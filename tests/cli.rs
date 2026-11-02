//! ximod-cli (D-06, T-D06-01 / 04 / 05 / 07 on the fixtures, run on the
//! three systems by the continuous integration): the executable itself,
//! its codes, its outputs, its JSON, the language of its messages, the
//! batch validated before anything is written, the configuration left
//! untouched — and the results equal to the library's.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ximod_translator::basedb::{self, BaseRow};
use ximod_translator::status::{self, Status};

const CLI: &str = env!("CARGO_BIN_EXE_ximod-cli");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// A configuration folder of its own and a work folder with a copy of the
/// plugin. The folder holds a `Config.ini` and an `assets/` tree, so the
/// command line takes it as a portable installation (R-D06-01): every base
/// it indexes is inside it, and the game bases of a developer's checkout
/// (`assets/bases/jeux` of the repository, read as the shipped bases in user
/// mode) never reach the tests.
struct Env {
    root: PathBuf,
    cfg: PathBuf,
    plugin: PathBuf,
}

impl Env {
    fn new(tag: &str) -> Env {
        let root = std::env::temp_dir().join(format!("ximod-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cfg = root.join("cfg");
        std::fs::create_dir_all(cfg.join("assets").join("bases").join("jeux")).unwrap();
        std::fs::create_dir_all(cfg.join("assets").join("bases").join("mods")).unwrap();
        std::fs::write(
            cfg.join("Config.ini"),
            "[General]\nLocale=fra\n[Translation]\nSourceLanguage=eng\nTargetLanguage=fra\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("w")).unwrap();
        let plugin = root.join("w").join("skyrim_dialogue.esp");
        std::fs::copy(fixture("skyrim_dialogue.esp"), &plugin).unwrap();
        Env { root, cfg, plugin }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(CLI)
            .arg("--config")
            .arg(&self.cfg)
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap()
    }

    /// A dictionary translating every line of the plugin (« FR: … »,
    /// Validé), as a translator would have made it.
    fn dict(&self) -> PathBuf {
        let tm = ximod_translator::plugin::read_file(&self.plugin).unwrap();
        let rows: Vec<BaseRow> = tm
            .entries
            .iter()
            .filter(|e| !e.unused && !e.source.trim().is_empty())
            .map(|e| {
                let mut e = e.clone();
                e.trans = format!("FR: {}", e.source);
                status::set(&mut e, Status::Validated);
                BaseRow::from_entry(&e, true)
            })
            .collect();
        let out = self.root.join("w").join("fr.ximdb");
        basedb::write_ximdb(rows, "fra", &out).unwrap();
        out
    }

    fn mod_dict(&self) -> PathBuf {
        self.cfg
            .join("assets/bases/mods/Skyrim")
            .join("skyrim_dialogue_ENG-FRA.ximdb")
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

fn json_lines(o: &Output) -> Vec<serde_json::Value> {
    text(&o.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn help_version_and_refusals() {
    let env = Env::new("help");
    let o = env.run(&["--help"]);
    assert_eq!(o.status.code(), Some(0));
    let help = text(&o.stdout);
    assert!(help.contains("Usage : ximod-cli"), "{help}");
    assert!(help.contains("batch"));
    let o = env.run(&["--lang", "deu", "export", "--help"]);
    assert!(text(&o.stdout).contains("Aufruf"), "{}", text(&o.stdout));
    let o = env.run(&["--version"]);
    assert!(text(&o.stdout).starts_with("XIMOD Translator "));
    // Unknown option, unknown command, missing file: 2, a message on stderr.
    let o = env.run(&["info", "x.esp", "--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("--bogus"));
    assert!(o.stdout.is_empty());
    let o = env.run(&["frobnicate"]);
    assert_eq!(o.status.code(), Some(2));
    let o = env.run(&["--json", "apply", "absent.esp"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("absent.esp"));
    let j = json_lines(&o);
    assert_eq!(j[0]["code"], 2);
    // translate refuses without --max-chars, before opening anything.
    let o = env.run(&[
        "translate",
        env.plugin.to_str().unwrap(),
        "--service",
        "deepl",
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("--max-chars"));
}

#[test]
fn info_in_json_and_in_german() {
    let env = Env::new("info");
    let p = env.plugin.to_str().unwrap();
    let o = env.run(&["--json", "info", p]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = &json_lines(&o)[0];
    assert_eq!(j["command"], "info");
    assert_eq!(j["kind"], "plugin");
    assert_eq!(j["game"], "Skyrim");
    assert_eq!(j["lines"], 23);
    assert_eq!(j["dictionary"]["state"], "none");
    assert_eq!(j["byType"]["INFO"], 9);
    let o = env.run(&["--lang", "deu", "info", p]);
    let t = text(&o.stdout);
    assert!(t.contains("Datei:") && t.contains("Mod-Wörterbuch"), "{t}");
}

#[test]
fn apply_export_manifest_package() {
    let env = Env::new("chain");
    let cfg_before = std::fs::read(env.cfg.join("Config.ini")).unwrap();
    let p = env.plugin.to_str().unwrap();
    let dict = env.dict();
    let o = env.run(&[
        "--json",
        "apply",
        p,
        "--dict",
        dict.to_str().unwrap(),
        "--write-dict",
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = &json_lines(&o)[0];
    assert_eq!(j["lines"], 23);
    assert_eq!(j["left"], 0);
    assert!(env.mod_dict().is_file());
    // The mod's dictionary: every line with its status (Validé on the
    // exact tier, Traduit par similarité on the text tier — the lines
    // without an EDID), the identity of the plugin (B-12).
    let rows = basedb::ximdb_to_tm(&env.mod_dict()).unwrap();
    assert_eq!(rows.entries.len(), 23);
    assert!(rows.entries.iter().all(|e| e.trans.starts_with("FR: ")));
    assert!(
        rows.entries
            .iter()
            .any(|e| status::derive(e) == Status::Validated)
    );
    let id = basedb::peek_ident(&env.mod_dict()).unwrap();
    assert_eq!(
        id.crc,
        crc32fast::hash(&std::fs::read(&env.plugin).unwrap())
    );
    // info sees it recognized.
    let o = env.run(&["--json", "info", p]);
    assert_eq!(json_lines(&o)[0]["dictionary"]["state"], "known");

    // export from the mod's dictionary: the override equal to the library's.
    let out = env.root.join("out");
    let o = env.run(&[
        "export",
        p,
        "--output",
        "override",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let written = std::fs::read(out.join("skyrim_dialogue_patch.esp")).unwrap();
    let mut tm = ximod_translator::plugin::read_file(&env.plugin).unwrap();
    let db = ximod_translator::basedb::FileDbCache::default()
        .get(&env.mod_dict())
        .unwrap();
    db.auto_translate_entries_norm(&mut tm.entries, None, None, |_, _, _| {});
    let opts = ximod_translator::plugin::OverrideOptions {
        bundle: None,
        esl_if_eligible: true,
    };
    // Validé only, the default policy (S-04).
    let policy = ximod_translator::export::ExportPolicy::validated_only();
    let filtered = policy.filtered(&tm);
    let (expected, _) = ximod_translator::plugin::write_override_esp_with(
        &std::fs::read(&env.plugin).unwrap(),
        &filtered,
        "skyrim_dialogue.esp",
        &opts,
    )
    .unwrap();
    assert!(written == expected, "the override is the library's");
    // The full plugin, under the same name, in the output folder only.
    let o = env.run(&["export", p, "--output", "full", "-o", out.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(out.join("skyrim_dialogue.esp").is_file());
    // Over the source itself: refused.
    let src_dir = env.plugin.parent().unwrap().to_str().unwrap().to_string();
    let o = env.run(&["export", p, "--output", "full", "-o", &src_dir]);
    assert_eq!(o.status.code(), Some(2));
    // DSD is for Skyrim SE: refused with a message, nothing written.
    let o = env.run(&["export", p, "--output", "dsd", "-o", out.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(!out.join("skyrim_dialogue.json").exists());

    // The manifest of the folder, the source plugin as input.
    let o = env.run(&["--json", "manifest", out.to_str().unwrap(), "--input", p]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let m = ximod_translator::manifest::Manifest::parse(
        &std::fs::read_to_string(out.join("manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m.outputs.len(), 2);
    assert_eq!(
        m.inputs[0].crc32,
        ximod_translator::manifest::crc_text(id.crc)
    );
    assert!(
        m.outputs
            .iter()
            .all(|o| o.kind == "plugin" && o.lines.is_some())
    );

    // The package: the plugins at the root, the manifest in fomod/.
    let zip = env.root.join("Dialogue-FR.zip");
    let o = env.run(&[
        "package",
        out.to_str().unwrap(),
        "--name",
        "Dialogue",
        "--version",
        "1.0",
        "--author",
        "Patrice",
        "-o",
        zip.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let z = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    let mut names: Vec<String> = z.file_names().map(str::to_string).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "fomod/info.xml",
            "fomod/manifest.json",
            "skyrim_dialogue.esp",
            "skyrim_dialogue_patch.esp"
        ]
    );
    // Nothing else left next to the zip.
    let left: Vec<_> = std::fs::read_dir(&env.root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(".ximod-package"))
        .collect();
    assert!(left.is_empty());

    // The configuration is untouched, no recent files, no autosave (T-D06-07).
    assert_eq!(
        std::fs::read(env.cfg.join("Config.ini")).unwrap(),
        cfg_before
    );
    assert!(!env.cfg.join("autosave").exists());
    assert!(!env.cfg.join("logs").exists());
}

#[test]
fn check_codes_and_report() {
    let env = Env::new("check");
    let p = env.plugin.to_str().unwrap();
    // A dictionary whose translations lose a number: errors.
    let tm = ximod_translator::plugin::read_file(&env.plugin).unwrap();
    let rows: Vec<BaseRow> = tm
        .entries
        .iter()
        .filter(|e| !e.unused && !e.source.trim().is_empty())
        .map(|e| {
            let mut e = e.clone();
            e.trans = format!("FR {}", e.source.replace(|c: char| c.is_ascii_digit(), ""));
            status::set(&mut e, Status::Validated);
            BaseRow::from_entry(&e, true)
        })
        .collect();
    let d = env.root.join("w").join("bad.ximdb");
    basedb::write_ximdb(rows, "fra", &d).unwrap();
    let report = env.root.join("check.csv");
    let o = env.run(&["--json", "check", p, "--report", report.to_str().unwrap()]);
    let code = o.status.code();
    let j = &json_lines(&o)[0];
    // Without the dictionary nothing is translated: no finding, code 0.
    assert_eq!(code, Some(0), "{j}");
    assert_eq!(j["errors"], 0);
    // With it (apply --dict … --write-dict, then check).
    let o = env.run(&["apply", p, "--dict", d.to_str().unwrap(), "--write-dict"]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let o = env.run(&["--json", "check", p, "--report", report.to_str().unwrap()]);
    let j = &json_lines(&o)[0];
    let errors = j["errors"].as_u64().unwrap();
    let warnings = j["warnings"].as_u64().unwrap();
    assert_eq!(o.status.code(), Some(if errors > 0 { 1 } else { 0 }), "{j}");
    let csv = std::fs::read_to_string(&report).unwrap();
    assert_eq!(
        csv.lines().count() as u64,
        1 + errors + warnings,
        "one line per finding"
    );
    // --fail-on warning: reservations when there are warnings.
    let o = env.run(&["check", p, "--fail-on", "warning"]);
    assert_eq!(
        o.status.code(),
        Some(if errors + warnings > 0 { 1 } else { 0 })
    );
}

#[test]
fn batch_is_validated_before_anything_is_written() {
    let env = Env::new("batch");
    let dict = env.dict();
    // Three plugins.
    let mods = env.root.join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    for n in ["a", "b", "c"] {
        std::fs::copy(&env.plugin, mods.join(format!("{n}.esp"))).unwrap();
    }
    let batch = |export: &str, stop: bool| {
        format!(
            r#"{{ "format": 1, "to": "fra", "out": "out", "stopOnError": {stop},
                "jobs": [ {{ "files": "mods/*.esp",
                  "steps": [ {{ "apply": {{ "dict": ["{}"] }} }}, "check",
                             {{ "export": {{ "output": "{export}" }} }}, "manifest" ] }} ] }}"#,
            dict.to_string_lossy().replace('\\', "/")
        )
    };
    let file = env.root.join("lot.json");
    std::fs::write(&file, batch("dsdx", false)).unwrap();
    let o = env.run(&["batch", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("dsdx"), "{}", text(&o.stderr));
    assert!(!env.root.join("out").exists(), "nothing written");
    // Corrected: the batch runs, one object per step with --json.
    std::fs::write(&file, batch("override", false)).unwrap();
    let o = env.run(&["--json", "batch", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = json_lines(&o);
    assert_eq!(j.len(), 1 + 3 * 4 + 1, "start, 12 steps, end");
    assert_eq!(j[0]["event"], "start");
    assert_eq!(j.last().unwrap()["ok"], 12);
    for n in ["a", "b", "c"] {
        let d = env.root.join("out").join(n);
        assert!(d.join(format!("{n}_patch.esp")).is_file());
        let m = ximod_translator::manifest::Manifest::parse(
            &std::fs::read_to_string(d.join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(m.inputs.len(), 1);
        assert_eq!(m.checks.unwrap().errors, 0);
        assert_eq!(m.stack.iter().map(|l| l.filled).sum::<usize>(), 23);
    }
    // A step that fails (DSD for Skyrim LE) gives 2: the batch goes on
    // without stopOnError, stops at the first one with it.
    std::fs::write(&file, batch("dsd", false)).unwrap();
    let o = env.run(&["--json", "batch", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    let end = json_lines(&o).last().unwrap().clone();
    assert_eq!(end["failed"], 3);
    std::fs::write(&file, batch("dsd", true)).unwrap();
    let o = env.run(&["--json", "batch", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    let end = json_lines(&o).last().unwrap().clone();
    assert_eq!(end["failed"], 1);
}

#[test]
fn compare_scan_convert_import() {
    let env = Env::new("misc");
    let dict = env.dict();
    let p = env.plugin.to_str().unwrap();
    // compare the dictionary with itself: everything identical.
    let csv = env.root.join("cmp.csv");
    let o = env.run(&[
        "--json",
        "compare",
        dict.to_str().unwrap(),
        dict.to_str().unwrap(),
        "--report",
        csv.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = &json_lines(&o)[0];
    assert_eq!(j["diff"], 0);
    assert_eq!(j["same"].as_u64().unwrap(), j["aLines"].as_u64().unwrap());
    assert!(csv.is_file());
    // scan the work folder.
    let o = env.run(&[
        "--json",
        "scan",
        env.plugin.parent().unwrap().to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = &json_lines(&o)[0];
    let files = j["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "the plugin and the dictionary");
    assert_eq!(j["totals"]["lines"], 23);
    // convert the dictionary to an xTranslator file and back.
    let sst = env.root.join("fr.sst");
    let o = env.run(&[
        "convert",
        dict.to_str().unwrap(),
        "-o",
        sst.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let back = env.root.join("back.ximdb");
    let o = env.run(&[
        "convert",
        sst.to_str().unwrap(),
        "-o",
        back.to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert_eq!(basedb::ximdb_to_tm(&back).unwrap().entries.len(), 23);
    // import a CSV table with a format (D-08).
    let o = env.run(&[
        "--json",
        "import",
        fixture("structured/table.csv").to_str().unwrap(),
        "--format",
        "csv-table",
        "-o",
        env.root.join("table.json").to_str().unwrap(),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(json_lines(&o)[0]["lines"].as_u64().unwrap() > 0);
    let o = env.run(&["import", p, "--format", "nope"]);
    assert_eq!(o.status.code(), Some(2));
}

/// A stand-in OpenAI-compatible server: « FR: text » for every text, the
/// Authorization headers it received kept.
fn mock_server(auth: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let mut body_at = None;
            loop {
                let n = match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..p]).to_string();
                    let lower = head.to_lowercase();
                    for l in head.lines() {
                        if l.to_lowercase().starts_with("authorization:") {
                            auth.lock().unwrap().push(l.to_string());
                        }
                    }
                    let len = lower
                        .lines()
                        .find_map(|l| {
                            l.strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    while buf.len() < p + 4 + len {
                        let n = match s.read(&mut tmp) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    body_at = Some(p + 4);
                    break;
                }
            }
            let Some(at) = body_at else { continue };
            let v: serde_json::Value =
                serde_json::from_slice(&buf[at..]).unwrap_or(serde_json::Value::Null);
            let user = v["messages"][1]["content"].as_str().unwrap_or("[]");
            let texts: Vec<String> = serde_json::from_str(user).unwrap_or_default();
            let answers: Vec<String> = texts.iter().map(|t| format!("FR: {t}")).collect();
            let content = serde_json::to_string(&answers).unwrap();
            let out =
                serde_json::json!({"choices":[{"message":{"role":"assistant","content":content}}]})
                    .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                out.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

#[test]
fn translate_through_a_service_and_the_key_never_shown() {
    let env = Env::new("translate");
    let auth = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let endpoint = mock_server(auth.clone());
    let dir = env.cfg.join("data").join("online").join("profiles");
    std::fs::create_dir_all(&dir).unwrap();
    let shipped =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/online/profiles/local.json");
    let mut p: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(shipped).unwrap()).unwrap();
    p["id"] = "mock".into();
    p["name"] = "Mock".into();
    p["endpoint"] = endpoint.into();
    p["model"] = "test".into();
    p["keyOptional"] = false.into();
    p["maxRpm"] = 600.into();
    std::fs::write(dir.join("mock.json"), p.to_string()).unwrap();
    let plugin = env.plugin.to_str().unwrap();
    let secret = "sk-ximod-secret-4711";
    let run = |args: &[&str], key: bool| {
        let mut c = Command::new(CLI);
        c.arg("--config")
            .arg(&env.cfg)
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"));
        if key {
            c.env("XIMOD_KEY_OPENAI_MOCK", secret);
        } else {
            c.env_remove("XIMOD_KEY_OPENAI_MOCK");
        }
        c.output().unwrap()
    };
    // Without a key: refused, nothing sent.
    let o = run(
        &[
            "translate",
            plugin,
            "--service",
            "openai:mock",
            "--max-chars",
            "100000",
        ],
        false,
    );
    assert_eq!(o.status.code(), Some(2), "{}", text(&o.stderr));
    assert!(text(&o.stderr).contains("XIMOD_KEY_OPENAI_MOCK"));
    // Beyond --max-chars: 1, nothing sent.
    let o = run(
        &[
            "--json",
            "translate",
            plugin,
            "--service",
            "openai:mock",
            "--max-chars",
            "10",
        ],
        true,
    );
    assert_eq!(o.status.code(), Some(1), "{}", text(&o.stderr));
    assert_eq!(json_lines(&o)[0]["sent"], false);
    assert!(auth.lock().unwrap().is_empty(), "nothing sent");
    // Within: every line received, En attente, the key used and never shown.
    let o = run(
        &[
            "--json",
            "--verbose",
            "translate",
            plugin,
            "--service",
            "openai:mock",
            "--max-chars",
            "100000",
            "--write-dict",
        ],
        true,
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = &json_lines(&o)[0];
    assert_eq!(j["received"], j["lines"], "{j}");
    assert!(j["received"].as_u64().unwrap() >= 20);
    assert!(!text(&o.stdout).contains(secret) && !text(&o.stderr).contains(secret));
    assert!(auth.lock().unwrap().iter().all(|a| a.contains(secret)));
    let rows = basedb::ximdb_to_tm(&env.mod_dict()).unwrap();
    assert!(
        rows.entries
            .iter()
            .all(|e| e.trans.starts_with("FR: ") && status::derive(e) == Status::Pending)
    );
    // The history of the operation, next to the dictionary (A-18).
    let hist =
        ximod_translator::history::read_file(&ximod_translator::history::file_for(&env.mod_dict()));
    assert_eq!(hist.len(), rows.entries.len());
    assert!(hist.iter().all(|h| h.op == "online"));
    // No log file, no key file written by the command line.
    assert!(!env.cfg.join("secrets.json").exists());
}
