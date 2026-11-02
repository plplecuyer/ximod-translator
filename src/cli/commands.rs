//! The commands of `ximod-cli` (R-D06-02): each opens what it needs, calls
//! the library and says what it did. The document commands are steps a
//! batch chains on the same open document (R-D06-03).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::args::Values;
use super::work::{Ctx, NotSent, Output, Pair, Source, Work};
use super::{FAIL, Fail, Global, Reply, Res, WARN};
use crate::headless::{Kind, TreeExtra};
use crate::l10n;
use crate::status::Status;

/// The game and languages of the session: the options, else the
/// configuration (R-D06-01).
pub fn pair(ctx: &Ctx, g: &Global) -> Pair {
    let pick = |o: &Option<String>, d: &str| {
        o.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(d.trim())
            .to_string()
    };
    Pair {
        game: pick(&g.game, &ctx.cfg.default_game),
        from: pick(&g.from, &ctx.cfg.source_language),
        to: pick(&g.to, &ctx.cfg.target_language),
    }
}

fn bad_value(arg: &str, value: &str) -> Fail {
    Fail(l10n!(
        "cli-err-value",
        arg = format!("--{arg}"),
        value = value
    ))
}

fn missing(arg: &str) -> Fail {
    Fail(l10n!("cli-err-missing", arg = format!("--{arg}")))
}

/// A message without arguments, translated when shown (crate::l10n).
fn key_msg(key: &str) -> String {
    crate::l10n::msg(key, &[])
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// « Mod.ximdb 812, SkyrimSE_ENG-FRA.ximdb 30 », « — » when nothing.
fn stack_detail(per_level: &[(String, usize)]) -> String {
    let v: Vec<String> = per_level
        .iter()
        .filter(|x| x.1 > 0)
        .map(|(n, k)| format!("{n} {k}"))
        .collect();
    if v.is_empty() {
        "—".to_string()
    } else {
        v.join(", ")
    }
}

pub fn parse_mode(s: &str) -> Option<crate::apply::Mode> {
    use crate::apply::Mode;
    Some(match s.trim() {
        "strict" => Mode::Strict,
        "relax" => Mode::Relax,
        "textonly" => Mode::TextOnly,
        "normalized" => Mode::Normalized,
        "formid" => Mode::FormId,
        "fuzzy" => Mode::Fuzzy,
        _ => return None,
    })
}

pub fn parse_overwrite(s: &str) -> Option<crate::apply::Overwrite> {
    use crate::apply::Overwrite;
    Some(match s.trim() {
        "none" => Overwrite::None,
        "keep-validated" => Overwrite::KeepValidated,
        "keep-pending" => Overwrite::KeepPending,
        "pending-only" => Overwrite::PendingOnly,
        "all" => Overwrite::All,
        _ => return None,
    })
}

/// `validated,pending,…` → the policy (S-04); Validé is always exported.
pub fn parse_statuses(s: &str) -> Option<crate::export::ExportPolicy> {
    let mut p = crate::export::ExportPolicy::validated_only();
    for k in s.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        let st = Status::from_key(k)?;
        if st == Status::Validated {
            continue;
        }
        if !crate::export::ExportPolicy::is_optional(st) {
            return None;
        }
        p = p.with(st, true);
    }
    Some(p)
}

/// The values of a command checked before anything is done (a batch
/// validates its steps with it).
pub fn validate(command: &str, opts: &Values) -> Result<(), Fail> {
    let check = |name: &str, ok: &dyn Fn(&str) -> bool| -> Result<(), Fail> {
        match opts.get(name) {
            Some(v) if !ok(v) => Err(bad_value(name, v)),
            _ => Ok(()),
        }
    };
    check("mode", &|v| parse_mode(v).is_some())?;
    check("overwrite", &|v| parse_overwrite(v).is_some())?;
    check("statuses", &|v| parse_statuses(v).is_some())?;
    check("fail-on", &|v| matches!(v, "error" | "warning"))?;
    check("scope", &|v| matches!(v, "untranslated" | "document"))?;
    check("max-chars", &|v| v.trim().parse::<usize>().is_ok())?;
    check("prefer", &|v| matches!(v, "a" | "b"))?;
    match command {
        "export" => {
            let Some(o) = opts.get("output") else {
                return Err(missing("output"));
            };
            if Output::from_id(o).is_none() {
                return Err(bad_value("output", o));
            }
        }
        "translate" => {
            if !opts.has("max-chars") {
                return Err(Fail(l10n!("cli-translate-needs-max")));
            }
            if let Some(s) = opts.get("service")
                && crate::online::Service::from_id(s).is_none()
            {
                return Err(bad_value("service", s));
            }
        }
        "compare" if opts.has("merge") && !opts.has("prefer") => {
            return Err(missing("prefer"));
        }
        "import" if !opts.has("format") => return Err(missing("format")),
        _ => {}
    }
    Ok(())
}

/// A command run alone.
pub fn run(ctx: &mut Ctx, g: &Global, command: &str, files: &[String], opts: &Values) -> Res {
    validate(command, opts)?;
    let pair = pair(ctx, g);
    let file = PathBuf::from(&files[0]);
    match command {
        "info" => info(ctx, &pair, &file),
        "import" => import(ctx, &pair, &file, opts),
        "apply" | "assist" | "translate" | "check" => {
            let extra = TreeExtra {
                dicts: opts.all("dict").into_iter().map(PathBuf::from).collect(),
                bases: opts.all("base").into_iter().map(PathBuf::from).collect(),
                no_personal: opts.has("no-personal"),
            };
            let mut w = Work::open(ctx, &file, &pair, Source::Tree(extra))?;
            step(ctx, &mut w, command, opts, None)
        }
        "export" => {
            let Some(dir) = opts.get("o") else {
                return Err(missing("o"));
            };
            let given = opts.get("dict").map(PathBuf::from);
            let mut w = Work::open(ctx, &file, &pair, Source::Only(given))?;
            step(ctx, &mut w, command, opts, Some(Path::new(dir)))
        }
        "manifest" => manifest_dir(&pair, &file, opts),
        "compare" => compare(ctx, &pair, &file, Path::new(&files[1]), opts),
        "scan" => scan(ctx, &pair, &file, opts),
        "convert" => convert(&pair, &file, opts),
        "package" => package(ctx, &pair, &file, opts, None),
        _ => Err(Fail(l10n!("cli-err-command", arg = command))),
    }
}

/// A command on an open document: alone or as a step of a batch (`dir`:
/// the folder of the outputs).
pub fn step(ctx: &mut Ctx, w: &mut Work, command: &str, opts: &Values, dir: Option<&Path>) -> Res {
    let mut r = match command {
        "info" => {
            let pair = Pair {
                game: w.o.settings.game.clone(),
                from: w.o.settings.source.clone(),
                to: w.o.settings.target.clone(),
            };
            let path = w.o.path.clone();
            info(ctx, &pair, &path)?
        }
        "apply" => apply(ctx, w, opts)?,
        "assist" => assist(ctx, w, opts)?,
        "translate" => translate(ctx, w, opts)?,
        "check" => check(ctx, w, opts)?,
        "export" => {
            let dir = opts
                .get("o")
                .map(PathBuf::from)
                .or_else(|| dir.map(Path::to_path_buf))
                .ok_or_else(|| missing("o"))?;
            export(w, opts, &dir)?
        }
        "manifest" => {
            let dir = dir
                .map(Path::to_path_buf)
                .unwrap_or_else(|| w.o.path.parent().map(Path::to_path_buf).unwrap_or_default());
            let out = opts
                .get("o")
                .map(PathBuf::from)
                .unwrap_or_else(|| dir.join(crate::manifest::FILE_NAME));
            let m = w.manifest(Some(&dir));
            write_manifest(&m, &out)?
        }
        "package" => {
            let dir = dir.map(Path::to_path_buf).ok_or_else(|| missing("o"))?;
            let pair = Pair {
                game: w.o.settings.game.clone(),
                from: w.o.settings.source.clone(),
                to: w.o.settings.target.clone(),
            };
            let plugin =
                w.o.path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string());
            package(ctx, &pair, &dir, opts, plugin)?
        }
        other => return Err(Fail(l10n!("cli-batch-step-unknown", step = other))),
    };
    if opts.has("write-dict") && matches!(command, "apply" | "assist" | "translate") {
        match w.write_dict(ctx) {
            Ok(n) => {
                r.line(l10n!(
                    "cli-dict-written",
                    lines = n,
                    format = crate::basedb::XIMDB_FORMAT,
                    path = path_text(&w.dict)
                ));
                r.set(
                    "dictionary",
                    json!({ "path": path_text(&w.dict), "lines": n,
                            "format": crate::basedb::XIMDB_FORMAT }),
                );
            }
            Err(e) => return Err(Fail(e)),
        }
    }
    r.set("file", path_text(&w.o.path));
    Ok(r)
}

// ---------------------------------------------------------------- info

fn kind_key(k: Kind) -> &'static str {
    match k {
        Kind::Plugin => "cli-kind-plugin",
        Kind::Text => "cli-kind-text",
        Kind::Dsd => "cli-kind-dsd",
        Kind::Script => "cli-kind-script",
        Kind::Dictionary => "cli-kind-dictionary",
    }
}

fn kind_id(k: Kind) -> &'static str {
    match k {
        Kind::Plugin => "plugin",
        Kind::Text => "text",
        Kind::Dsd => "dsd",
        Kind::Script => "script",
        Kind::Dictionary => "dictionary",
    }
}

fn dict_key(d: crate::scan::DictState) -> &'static str {
    use crate::scan::DictState as D;
    match d {
        D::Known => "scan-dict-known",
        D::Other => "scan-dict-other",
        D::Absent => "scan-dict-none",
        D::Is => "scan-dict-is",
    }
}

/// `info`: game, languages detected, lines per type, the mod's
/// dictionary, the interface files found.
fn info(ctx: &mut Ctx, pair: &Pair, file: &Path) -> Res {
    if !file.is_file() {
        return Err(Fail(l10n!("cli-err-file", path = file.display())));
    }
    let kind = crate::headless::kind_of(file, &ctx.formats)
        .ok_or_else(|| Fail(l10n!("cli-err-kind", path = file.display())))?;
    let o = crate::headless::read(
        file,
        kind,
        &pair.game,
        &pair.from,
        &pair.to,
        &ctx.cfg,
        &ctx.formats,
    )
    .map_err(|e| Fail(e.to_string()))?;
    let mut r = Reply::new("info");
    let lines: Vec<&crate::tm::TmEntry> =
        o.tm.entries
            .iter()
            .filter(|e| !e.unused && !e.source.trim().is_empty())
            .collect();
    let mut by: std::collections::BTreeMap<String, usize> = Default::default();
    for e in &lines {
        *by.entry(e.rec_sig.clone()).or_default() += 1;
    }
    let langs = crate::langdetect::detect_texts(lines.iter().map(|e| e.source.as_str()));
    let dict =
        crate::dict_stack::DictStack::mod_dict_path(file, &o.settings, &ctx.cfg.mod_base_dir());
    let ident = crate::plugin_id::of_file(file).ok();
    let state = if matches!(kind, Kind::Dsd | Kind::Dictionary) {
        crate::scan::DictState::Is
    } else if !o.settings.target.trim().is_empty() && dict.is_file() {
        match (crate::basedb::peek_ident(&dict), &ident) {
            (Some(d), Some(f)) if !d.same_content(f) => crate::scan::DictState::Other,
            _ => crate::scan::DictState::Known,
        }
    } else {
        crate::scan::DictState::Absent
    };
    let interface = if kind == Kind::Plugin {
        crate::textfmt::interface_files_of(file, &o.settings.source, o.settings.game())
    } else {
        Vec::new()
    };
    let game = o.settings.game.trim().to_string();
    r.line(l10n!("cli-info-file", path = path_text(file)));
    r.line(l10n!("cli-info-kind", kind = key_msg(kind_key(kind))));
    r.line(l10n!(
        "cli-info-game",
        game = if game.is_empty() { "—" } else { &game }
    ));
    let lang_text: Vec<String> = langs
        .iter()
        .take(3)
        .map(|c| format!("{} {:.0} %", c.iso3, c.confidence * 100.0))
        .collect();
    r.line(l10n!(
        "cli-info-langs",
        langs = if lang_text.is_empty() {
            "—".to_string()
        } else {
            lang_text.join(", ")
        }
    ));
    let detail: Vec<String> = by.iter().map(|(k, n)| format!("{k} {n}")).collect();
    r.line(l10n!(
        "cli-info-lines",
        lines = lines.len(),
        detail = detail.join(", ")
    ));
    r.line(l10n!(
        "cli-info-dict",
        state = key_msg(dict_key(state)),
        path = path_text(&dict)
    ));
    if let Some(id) = &ident {
        r.line(l10n!(
            "cli-info-crc",
            size = id.size,
            crc = crate::manifest::crc_text(id.crc)
        ));
    }
    let names: Vec<String> = interface.iter().map(|p| path_text(p)).collect();
    r.line(l10n!(
        "cli-info-interface",
        files = if names.is_empty() {
            "—".to_string()
        } else {
            names.join(", ")
        }
    ));
    r.set("kind", kind_id(kind));
    r.set("game", game);
    r.set(
        "langs",
        Value::Array(
            langs
                .iter()
                .map(|c| json!({ "lang": c.iso3, "confidence": c.confidence }))
                .collect(),
        ),
    );
    r.set("lines", lines.len());
    r.set("byType", json!(by));
    r.set(
        "dictionary",
        json!({ "state": state.code(), "path": path_text(&dict) }),
    );
    if let Some(id) = &ident {
        r.set("size", id.size);
        r.set("crc32", crate::manifest::crc_text(id.crc));
    }
    r.set("interface", json!(names));
    r.set("file", path_text(file));
    Ok(r)
}

// --------------------------------------------------------------- import

/// `import`: a text, JSON or XML file read with a format (D-08), written
/// as a pivot or a dictionary.
fn import(ctx: &mut Ctx, pair: &Pair, file: &Path, opts: &Values) -> Res {
    if !file.is_file() {
        return Err(Fail(l10n!("cli-err-file", path = file.display())));
    }
    let id = opts.get("format").unwrap_or_default();
    let fmt = ctx
        .formats
        .formats
        .iter()
        .find(|f| f.id == id)
        .cloned()
        .ok_or_else(|| Fail(l10n!("cli-err-format", id = id)))?;
    let data = std::fs::read(file)
        .map_err(|e| Fail(l10n!("err-write-path", path = file.display(), error = e)))?;
    let doc = crate::textfmt::TextDoc::parse(&data, &fmt).map_err(|e| Fail(e.to_string()))?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let tm = doc.to_tm(&fmt, &name, &pair.game);
    let out = opts.get("o").map(PathBuf::from).unwrap_or_else(|| {
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        file.with_file_name(format!("{stem}.ximod.json"))
    });
    let n = if crate::basedb::is_ximdb_path(&out) {
        if pair.to.is_empty() {
            return Err(Fail(l10n!("dict-stack-no-target")));
        }
        let rows: Vec<crate::basedb::BaseRow> = tm
            .entries
            .iter()
            .filter(|e| !e.unused && e.is_translated())
            .map(|e| crate::basedb::BaseRow::from_entry(e, true))
            .collect();
        crate::basedb::write_ximdb(rows, &pair.to, &out).map_err(Fail)?
    } else {
        crate::tm::save(&tm, &out).map_err(|e| Fail(e.to_string()))?;
        tm.entries.len()
    };
    let mut r = Reply::new("import");
    r.line(l10n!(
        "cli-import-done",
        lines = n,
        format = fmt.id,
        path = path_text(&out)
    ));
    r.set("format", fmt.id.clone());
    r.set("lines", n);
    r.set("output", path_text(&out));
    r.set("file", path_text(file));
    Ok(r)
}

// ---------------------------------------------------------------- apply

/// `apply`: the stack applied at the opening, then, with `--mode` or
/// `--overwrite`, Appliquer les dictionnaires on the whole document.
fn apply(ctx: &mut Ctx, w: &mut Work, opts: &Values) -> Res {
    w.needs_target().map_err(Fail)?;
    let mut r = Reply::new("apply");
    let a = &w.applied;
    r.set(
        "stack",
        serde_json::to_value(w.levels()).unwrap_or_default(),
    );
    r.set("filledAtOpening", a.filled);
    if opts.has("mode") || opts.has("overwrite") {
        let o = crate::apply::Options {
            mode: opts.get("mode").and_then(parse_mode).unwrap_or_default(),
            overwrite: opts
                .get("overwrite")
                .and_then(parse_overwrite)
                .unwrap_or_default(),
            normalize: ctx.cfg.normalize,
            ..Default::default()
        };
        let plan = w.apply_mode(ctx, &o);
        r.line(l10n!(
            "cli-apply-mode",
            mode = opts.get("mode").unwrap_or("strict"),
            n = plan.filled(),
            over = plan.overwritten
        ));
        r.set("filled", plan.filled());
        r.set("overwritten", plan.overwritten);
        r.set("ambiguous", plan.ambiguous);
    }
    let (lines, left, locked) = w.counts();
    r.lines.insert(
        0,
        l10n!(
            "cli-apply-done",
            lines = lines,
            stack = stack_detail(&w.applied.report.per_level),
            left = left,
            locked = locked
        ),
    );
    r.set("lines", lines);
    r.set("left", left);
    r.set("locked", locked);
    r.set("dictionaryState", w.dict_state.code());
    Ok(r)
}

// --------------------------------------------------------------- assist

fn assist(ctx: &mut Ctx, w: &mut Work, opts: &Values) -> Res {
    w.needs_target().map_err(Fail)?;
    let any = opts.has("glossary") || opts.has("patterns") || opts.has("derive");
    let o = crate::assist::Options {
        glossary: !any || opts.has("glossary"),
        patterns: !any || opts.has("patterns"),
        derive: !any || opts.has("derive"),
        overwrite: opts
            .get("overwrite")
            .and_then(parse_overwrite)
            .unwrap_or_default(),
    };
    let plan = w.assist(ctx, &o);
    let mut r = Reply::new("assist");
    let detail = l10n!(
        "cli-assist-detail",
        composed = plan.composed,
        glossary = plan.glossary,
        patterns = plan.patterned(),
        derived = plan.derived_n()
    );
    r.line(l10n!("cli-assist-done", n = plan.filled(), detail = detail));
    r.set("filled", plan.filled());
    r.set("composed", plan.composed);
    r.set("glossary", plan.glossary);
    r.set("patterns", plan.patterned());
    r.set("derived", json!(plan.derived));
    r.set("overwritten", plan.overwritten);
    Ok(r)
}

// ------------------------------------------------------------ translate

fn translate(ctx: &mut Ctx, w: &mut Work, opts: &Values) -> Res {
    w.needs_target().map_err(Fail)?;
    let max: usize = opts
        .get("max-chars")
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| Fail(l10n!("cli-translate-needs-max")))?;
    let service = match opts.get("service") {
        Some(s) => crate::online::Service::from_id(s).ok_or_else(|| bad_value("service", s))?,
        None => ctx
            .cfg
            .online
            .service
            .clone()
            .ok_or_else(|| missing("service"))?,
    };
    if let crate::online::Service::OpenAI(id) = &service
        && crate::online::profile::get(id).is_none()
    {
        return Err(bad_value("service", &service.id()));
    }
    let key = match super::work::service_key(&service) {
        Some(k) => k,
        None if service.key_optional() => String::new(),
        None => {
            return Err(Fail(l10n!(
                "cli-translate-no-key",
                service = service.id(),
                var = super::work::key_var(&service)
            )));
        }
    };
    let only = opts.get("scope") != Some("document");
    let mut r = Reply::new("translate");
    r.set("service", service.id());
    match w.translate(ctx, &service, key, only, max) {
        Err(NotSent::Over(chars, max)) => {
            r.code = WARN;
            r.line(l10n!("cli-translate-over", chars = chars, max = max));
            r.set("chars", chars);
            r.set("maxChars", max);
            r.set("sent", false);
        }
        Err(NotSent::Nothing) => {
            r.line(l10n!("online-nothing"));
            r.set("sent", false);
        }
        Ok(t) => {
            let s = &t.summary;
            r.line(l10n!(
                "cli-translate-done",
                items = t.items,
                chars = t.chars,
                received = s.received,
                refused = s.refused,
                left = s.left
            ));
            for (reason, n) in &s.by_reason {
                if let crate::online::mass::Reason::Service { code, message } = reason {
                    r.notes.push(l10n!(
                        "online-refused-by",
                        n = n,
                        reason = l10n!("online-refused-error", code = code, message = message)
                    ));
                }
            }
            let unreachable = s.received == 0
                && s.by_reason
                    .keys()
                    .any(|k| matches!(k, crate::online::mass::Reason::Service { .. }));
            if unreachable {
                r.code = FAIL;
            } else if t.stopped {
                r.worse(WARN);
            }
            r.set("sent", true);
            r.set("lines", t.lines);
            r.set("items", t.items);
            r.set("cached", t.cached);
            r.set("chars", t.chars);
            r.set("received", s.received);
            r.set("refused", s.refused);
            r.set("left", s.left);
            r.set(
                "skipped",
                json!({ "locked": t.skipped.locked, "done": t.skipped.done }),
            );
        }
    }
    Ok(r)
}

// ---------------------------------------------------------------- check

fn check(ctx: &mut Ctx, w: &mut Work, opts: &Values) -> Res {
    let report = w.check(ctx);
    let c = w.checks.unwrap_or_default();
    let mut r = Reply::new("check");
    r.line(l10n!(
        "cli-check-done",
        warnings = c.warnings,
        errors = c.errors
    ));
    let flat = |t: &str| {
        let s: String = t.split(['\r', '\n']).collect::<Vec<_>>().join(" / ");
        let mut out: String = s.chars().take(40).collect();
        if s.chars().count() > 40 {
            out.push('…');
        }
        out
    };
    let mut rows = Vec::new();
    let mut csv = String::from(
        "\u{feff}line;record;field;formid;edid;level;check;message;source;translation\r\n",
    );
    for f in &report.findings {
        let Some(e) = w.o.tm.entries.get(f.line) else {
            continue;
        };
        let error = super::work::is_error(f.check);
        let text = crate::quality::finding_text(&ctx.i18n, f, flat);
        let level = if error { "error" } else { "warning" };
        let id = format!("{}:{} {:08X} {}", e.rec_sig, e.field_sig, e.formid, e.edid);
        r.line(l10n!(
            "cli-check-line",
            level = key_msg(if error {
                "cli-level-error"
            } else {
                "cli-level-warning"
            }),
            line = f.line + 1,
            id = id.trim(),
            text = text
        ));
        use crate::scan::csv_cell as cell;
        let cells = [
            (f.line + 1).to_string(),
            cell(&e.rec_sig),
            cell(&e.field_sig),
            format!("{:08X}", e.formid),
            cell(&e.edid),
            level.to_string(),
            f.check.key().to_string(),
            cell(&text),
            cell(&e.source),
            cell(&e.trans),
        ];
        csv.push_str(&cells.join(";"));
        csv.push_str("\r\n");
        rows.push(json!({
            "line": f.line + 1, "record": e.rec_sig, "field": e.field_sig,
            "formid": format!("{:08X}", e.formid), "edid": e.edid,
            "level": level, "check": f.check.key(), "message": text,
        }));
    }
    if let Some(p) = opts.get("report") {
        let p = PathBuf::from(p);
        crate::config::paths::write_atomic(&p, csv.as_bytes())
            .map_err(|e| Fail(l10n!("err-write-path", path = p.display(), error = e)))?;
        r.line(l10n!("scan-exported", path = path_text(&p)));
        r.set("report", path_text(&p));
    }
    let fail_on_warning = opts.get("fail-on") == Some("warning");
    if c.errors > 0 || (fail_on_warning && c.warnings > 0) {
        r.code = WARN;
    }
    r.set("checked", report.lines_checked);
    r.set("errors", c.errors);
    r.set("warnings", c.warnings);
    r.set("findings", Value::Array(rows));
    Ok(r)
}

// --------------------------------------------------------------- export

fn export(w: &mut Work, opts: &Values, dir: &Path) -> Res {
    let output = opts
        .get("output")
        .and_then(Output::from_id)
        .ok_or_else(|| missing("output"))?;
    let policy = match opts.get("statuses") {
        Some(s) => parse_statuses(s).ok_or_else(|| bad_value("statuses", s))?,
        None => crate::export::ExportPolicy::validated_only(),
    };
    let written = w
        .export(output, policy, dir, !opts.has("no-esl"))
        .map_err(Fail)?;
    let mut r = Reply::new("export");
    let mut files = Vec::new();
    for (p, n) in &written {
        r.line(l10n!("cli-export-done", path = path_text(p), lines = n));
        files.push(json!({ "path": path_text(p), "lines": n }));
    }
    let left = w.untranslated(&policy);
    if opts.has("strict") && left > 0 {
        r.code = WARN;
        r.notes.push(l10n!("cli-export-left", n = left));
    }
    r.set("output", output.id());
    r.set("statuses", json!(w.statuses));
    r.set("outputs", Value::Array(files));
    r.set("untranslated", left);
    Ok(r)
}

// ------------------------------------------------------------- manifest

fn write_manifest(m: &crate::manifest::Manifest, out: &Path) -> Res {
    if let Some(d) = out.parent()
        && !d.as_os_str().is_empty()
    {
        let _ = std::fs::create_dir_all(d);
    }
    crate::config::paths::write_atomic(out, m.to_json().as_bytes())
        .map_err(|e| Fail(l10n!("err-write-path", path = out.display(), error = e)))?;
    let mut r = Reply::new("manifest");
    r.line(l10n!(
        "cli-manifest-done",
        files = m.outputs.len(),
        lines = m.output_lines(),
        crc = m.source_crc().unwrap_or("—")
    ));
    r.set("path", path_text(out));
    r.set("manifest", serde_json::to_value(m).unwrap_or_default());
    Ok(r)
}

/// `manifest <dossier>`: the outputs found described.
fn manifest_dir(pair: &Pair, dir: &Path, opts: &Values) -> Res {
    if !dir.is_dir() {
        return Err(Fail(l10n!("cli-err-file", path = dir.display())));
    }
    let mut m = crate::manifest::of_dir(dir, &pair.game, &pair.from, &pair.to);
    // The files the outputs were made from (`--input`): the source plugin
    // first, its size and CRC-32 (B-12).
    for i in opts.all("input") {
        let p = PathBuf::from(&i);
        let input = crate::manifest::input(&p, None)
            .map_err(|_| Fail(l10n!("cli-err-file", path = p.display())))?;
        m.inputs.push(input);
    }
    let out = opts
        .get("o")
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join(crate::manifest::FILE_NAME));
    write_manifest(&m, &out)
}

// -------------------------------------------------------------- compare

fn compare(ctx: &mut Ctx, pair: &Pair, a: &Path, b: &Path, opts: &Values) -> Res {
    use crate::compare::{self as cmp, Category, Mode, Prefer};
    for p in [a, b] {
        if !p.is_file() {
            return Err(Fail(l10n!("cli-err-file", path = p.display())));
        }
    }
    let ta = cmp::read_side(a, &pair.from).map_err(Fail)?;
    let tb = cmp::read_side(b, &pair.from).map_err(Fail)?;
    let mode = if opts.get("mode") == Some("text") {
        Mode::Text
    } else {
        Mode::Record
    };
    if let Some(m) = opts.get("mode")
        && !matches!(m, "record" | "text")
    {
        return Err(bad_value("mode", m));
    }
    let rep = cmp::compare(&ta, &tb, mode, opts.has("fold"));
    let name = |p: &Path| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    let (na, nb) = (name(a), name(b));
    let (same, diff) = (rep.count(Category::Same), rep.count(Category::Diff));
    let (oa, ob) = (rep.count(Category::OnlyA), rep.count(Category::OnlyB));
    let mut r = Reply::new("compare");
    r.line(l10n!(
        "compare-summary",
        a = rep.a_lines,
        b = rep.b_lines,
        common = same + diff,
        same = same,
        diff = diff,
        onlyA = oa,
        onlyB = ob
    ));
    if mode == Mode::Record && rep.mode == Some(Mode::Text) {
        r.notes.push(l10n!("compare-text-fallback"));
    }
    if let Some(p) = opts.get("report") {
        let p = PathBuf::from(p);
        crate::config::paths::write_atomic(&p, cmp::to_csv(&rep, &na, &nb).as_bytes())
            .map_err(|e| Fail(l10n!("err-write-path", path = p.display(), error = e)))?;
        r.line(l10n!("scan-exported", path = path_text(&p)));
        r.set("report", path_text(&p));
    }
    if let Some(m) = opts.get("merge") {
        if pair.to.is_empty() {
            return Err(Fail(l10n!("dict-stack-no-target")));
        }
        let prefer = if opts.get("prefer") == Some("b") {
            Prefer::B
        } else {
            Prefer::A
        };
        let out = PathBuf::from(m);
        let (rows, amb) = cmp::merge(&rep, prefer, &name(&out));
        let n = cmp::write_merge(rows, &pair.to, &out).map_err(Fail)?;
        ctx.cache.invalidate(Some(&out));
        r.line(l10n!(
            "compare-merged",
            path = path_text(&out),
            n = n,
            amb = amb
        ));
        r.set(
            "merge",
            json!({ "path": path_text(&out), "lines": n, "ambiguous": amb }),
        );
    }
    r.set("a", path_text(a));
    r.set("b", path_text(b));
    r.set("aLines", rep.a_lines);
    r.set("bLines", rep.b_lines);
    r.set("same", same);
    r.set("diff", diff);
    r.set("onlyA", oa);
    r.set("onlyB", ob);
    r.set(
        "rows",
        Value::Array(
            rep.rows
                .iter()
                .map(|x| {
                    json!({ "category": x.category.code(), "record": x.rec, "field": x.field,
                            "edid": x.edid, "source": x.source, "a": x.a, "b": x.b })
                })
                .collect(),
        ),
    );
    Ok(r)
}

// ----------------------------------------------------------------- scan

fn scan(ctx: &mut Ctx, pair: &Pair, dir: &Path, opts: &Values) -> Res {
    if !dir.is_dir() {
        return Err(Fail(l10n!("cli-err-file", path = dir.display())));
    }
    let so = crate::scan::Options {
        subdirs: !opts.has("no-subdirs"),
        scripts: opts.has("scripts"),
        ..Default::default()
    };
    let t0 = std::time::Instant::now();
    let files = crate::scan::list(dir, so, &ctx.formats);
    let mut sc = crate::scan::Scanner::new(ctx.cfg.clone(), dir, &pair.game, &pair.from, &pair.to);
    let reports: Vec<crate::scan::FileReport> =
        files.into_iter().map(|(p, k)| sc.analyze(&p, k)).collect();
    let secs = t0.elapsed().as_secs_f32();
    let t = crate::scan::totals(&reports);
    let mut r = Reply::new("scan");
    for f in &reports {
        r.line(l10n!(
            "cli-scan-file",
            state = key_msg(&format!("scan-state-{}", f.state.code())),
            file = f.rel,
            filled = f.filled,
            lines = f.lines
        ));
    }
    r.line(l10n!(
        "scan-summary",
        files = t.files,
        secs = format!("{secs:.1}"),
        lines = t.lines,
        done = t.filled,
        left = t.left,
        locked = t.locked
    ));
    if let Some(p) = opts.get("report") {
        let p = PathBuf::from(p);
        crate::config::paths::write_atomic(&p, crate::scan::to_csv(&reports).as_bytes())
            .map_err(|e| Fail(l10n!("err-write-path", path = p.display(), error = e)))?;
        r.line(l10n!("scan-exported", path = path_text(&p)));
        r.set("report", path_text(&p));
    }
    r.set("dir", path_text(dir));
    r.set(
        "totals",
        json!({ "files": t.files, "lines": t.lines, "translated": t.filled,
                "left": t.left, "locked": t.locked }),
    );
    r.set(
        "files",
        Value::Array(
            reports
                .iter()
                .map(|f| {
                    json!({ "file": f.rel, "game": f.game, "lines": f.lines,
                            "translated": f.filled, "left": f.left, "locked": f.locked,
                            "dictionary": f.dict.code(), "state": f.state.code(),
                            "error": f.error })
                })
                .collect(),
        ),
    );
    Ok(r)
}

// -------------------------------------------------------------- convert

/// `convert`: the conversions of the Dictionnaires menu — EET, SST, XML,
/// pivot ↔ ximdb, and between the formats of translations.
fn convert(pair: &Pair, input: &Path, opts: &Values) -> Res {
    if !input.is_file() {
        return Err(Fail(l10n!("cli-err-file", path = input.display())));
    }
    let Some(out) = opts.get("o").map(PathBuf::from) else {
        return Err(missing("o"));
    };
    let n = if crate::basedb::is_ximdb_path(&out) {
        if pair.to.is_empty() {
            return Err(Fail(l10n!("dict-stack-no-target")));
        }
        crate::basedb::compile_files(&[input.to_path_buf()], &pair.to, &out)
            .map_err(Fail)?
            .1
    } else if crate::basedb::is_ximdb_path(input) {
        crate::basedb::export_ximdb(input, &out).map_err(Fail)?
    } else {
        crate::tm::convert(input, &out)
            .map_err(|e| Fail(e.to_string()))?
            .0
    };
    let mut r = Reply::new("convert");
    r.line(l10n!("cli-convert-done", lines = n, path = path_text(&out)));
    r.set("input", path_text(input));
    r.set("output", path_text(&out));
    r.set("lines", n);
    Ok(r)
}

// -------------------------------------------------------------- package

/// `package` from a batch.
pub fn run_package(
    ctx: &Ctx,
    pair: &Pair,
    dir: &Path,
    opts: &Values,
    plugin: Option<String>,
) -> Res {
    package(ctx, pair, dir, opts, plugin)
}

/// `package`: the outputs of a folder laid out for the mod managers
/// (M-03), the manifest in `fomod/`, zipped.
fn package(ctx: &Ctx, pair: &Pair, dir: &Path, opts: &Values, plugin: Option<String>) -> Res {
    if !dir.is_dir() {
        return Err(Fail(l10n!("cli-err-file", path = dir.display())));
    }
    let manifest = std::fs::read_to_string(dir.join(crate::manifest::FILE_NAME))
        .ok()
        .and_then(|t| crate::manifest::Manifest::parse(&t).ok());
    let mut files: Vec<PathBuf> = crate::manifest::files_in(dir)
        .into_iter()
        .filter(|p| !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect();
    if manifest.is_some() {
        files.push(dir.join(crate::manifest::FILE_NAME));
    }
    let game_id = if pair.game.is_empty() {
        manifest
            .as_ref()
            .map(|m| m.game.clone())
            .unwrap_or_default()
    } else {
        pair.game.clone()
    };
    let lang = if pair.to.is_empty() {
        manifest.as_ref().map(|m| m.to.clone()).unwrap_or_default()
    } else {
        pair.to.clone()
    };
    let dsd_plugin = opts
        .get("plugin")
        .map(str::to_string)
        .or(plugin)
        .or_else(|| {
            manifest
                .as_ref()
                .and_then(|m| m.inputs.first())
                .map(|i| i.path.rsplit('/').next().unwrap_or("").to_string())
        })
        .unwrap_or_default();
    let name = opts.get("name").map(str::to_string).unwrap_or_else(|| {
        Path::new(&dsd_plugin)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let zip = opts.get("o").map(PathBuf::from);
    let tmp = zip.as_ref().map(|z| {
        z.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .join(format!(".ximod-package-{}", std::process::id()))
    });
    let out_dir = tmp.clone().unwrap_or_else(|| {
        dir.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    let spec = crate::packaging::PackageSpec {
        game: crate::games::Game::from_id(&game_id),
        mod_name: name,
        version: opts.get("version").unwrap_or_default().to_string(),
        author: opts.get("author").unwrap_or_default().to_string(),
        lang,
        website: opts.get("website").unwrap_or_default().to_string(),
        files,
        dsd_plugin,
        out_dir: out_dir.clone(),
        with_fomod_info: !opts.has("no-info"),
    };
    let plan = crate::packaging::plan(&spec);
    let description = match opts.get("description") {
        Some(d) if !d.trim().is_empty() => d.to_string(),
        _ => ctx.i18n.t_args(
            "pack-default-description",
            &[("mod", &spec.mod_name), ("lang", &spec.lang.to_uppercase())],
        ),
    };
    let built = crate::packaging::build(&spec, &description).map_err(|e| Fail(e.to_string()));
    let (folder, made) = match built {
        Ok(x) => x,
        Err(f) => {
            if let Some(t) = &tmp {
                let _ = std::fs::remove_dir_all(t);
            }
            return Err(f);
        }
    };
    let zip_path = match (&zip, &tmp) {
        (Some(z), Some(t)) => {
            let moved = std::fs::rename(&made, z).or_else(|_| std::fs::copy(&made, z).map(|_| ()));
            let _ = std::fs::remove_dir_all(t);
            moved.map_err(|e| Fail(l10n!("err-write-path", path = z.display(), error = e)))?;
            z.clone()
        }
        _ => made,
    };
    let mut r = Reply::new("package");
    r.line(l10n!(
        "cli-package-done",
        path = path_text(&zip_path),
        files = plan.copies.len()
    ));
    for k in &plan.notes {
        r.notes.push(key_msg(k));
    }
    for u in &plan.unknown {
        r.notes.push(l10n!("cli-package-left", file = path_text(u)));
    }
    r.set("path", path_text(&zip_path));
    if zip.is_none() {
        r.set("folder", path_text(&folder));
    }
    r.set(
        "files",
        Value::Array(
            plan.copies
                .iter()
                .map(|c| Value::from(c.dest.clone()))
                .collect(),
        ),
    );
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_checked() {
        let mut v = Values::default();
        v.set("output", "dsdx");
        assert!(validate("export", &v).is_err());
        let mut v = Values::default();
        v.set("output", "dsd");
        v.set("statuses", "validated,pending");
        assert!(validate("export", &v).is_ok());
        v.set("statuses", "to_translate");
        assert!(validate("export", &v).is_err());
        assert!(validate("translate", &Values::default()).is_err());
        let mut v = Values::default();
        v.set("mode", "fuzzy");
        v.set("overwrite", "keep-validated");
        assert!(validate("apply", &v).is_ok());
        assert_eq!(super::super::OK, 0);
    }
}
