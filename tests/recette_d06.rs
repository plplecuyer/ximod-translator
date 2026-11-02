//! D-06 recette on the mods of lot 4 (T-D06-01 to 04, 07), run by hand:
//! `XIMOD_RECETTE_D06=<recette_lot4> XIMOD_D06_WORK=<work folder> cargo
//! test --release --test recette_d06 -- --ignored --nocapture`.
//!
//! The translator's work is made first, as the interface would have left
//! it: the mods' dictionaries of Apocalypse (its French plugin), CACO and
//! Immersive Weapons (their French DSD), every line Validé. Then the
//! command line alone: the script of `docs/cli/` rebuilds Apocalypse; the
//! assistants on the scrolls and tomes (B-21); translate through a
//! stand-in service; a batch of the three mods.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;

const CLI: &str = env!("CARGO_BIN_EXE_ximod-cli");

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

fn json(o: &Output) -> Vec<serde_json::Value> {
    text(&o.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

struct R {
    cfg: PathBuf,
    work: PathBuf,
}

impl R {
    fn cli(&self, args: &[&str]) -> Output {
        Command::new(CLI)
            .arg("--config")
            .arg(&self.cfg)
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap()
    }

    fn mods(&self) -> PathBuf {
        self.cfg.join("data/bases/mods/SkyrimSE")
    }
}

fn plugin(p: &Path) -> ximod_translator::tm::TranslationMemory {
    ximod_translator::plugin::read_file_as(p, Some(ximod_translator::games::Game::SkyrimSE))
        .unwrap()
}

type Key = (String, u32, String, i32);

fn key(e: &TmEntry) -> Key {
    (e.rec_sig.clone(), e.formid, e.field_sig.clone(), e.index)
}

/// Write the mod's dictionary of `en` with the translations `of`, Validé
/// (a translation equal to the source left aside), as the interface writes
/// it; the lines written.
fn mod_dict(r: &R, en: &Path, of: &dyn Fn(&TmEntry) -> Option<String>) -> usize {
    mod_dict_in(&r.mods(), en, of)
}

fn mod_dict_in(mods: &Path, en: &Path, of: &dyn Fn(&TmEntry) -> Option<String>) -> usize {
    let mut tm = plugin(en);
    for e in tm.entries.iter_mut() {
        if let Some(t) = of(e)
            && !t.trim().is_empty()
            && t != e.source
        {
            e.trans = t;
            status::set(e, Status::Validated);
        }
    }
    let stem = en.file_stem().unwrap().to_string_lossy().to_string();
    let out = mods.join(format!("{stem}_ENG-FRA.ximdb"));
    let ident = ximod_translator::plugin_id::of_file(en).unwrap();
    ximod_translator::mod_dict::write(&ximod_translator::mod_dict::Write {
        entries: &tm.entries,
        rules: Vec::new(),
        rules_off: Vec::new(),
        enc_in: None,
        enc_out: None,
        lang: "fra",
        out: &out,
        ident: Some(&ident),
        backup_copies: 0,
        old_months: 0,
    })
    .unwrap()
}

/// The translations of a DSD by record:field and source, then by source.
fn dsd_of(path: &Path) -> impl Fn(&TmEntry) -> Option<String> {
    let dsd = ximod_translator::tm::load(path).unwrap();
    let mut by: HashMap<(String, String), String> = HashMap::new();
    let mut by_src: HashMap<String, String> = HashMap::new();
    for e in &dsd.entries {
        by.insert(
            (format!("{}:{}", e.rec_sig, e.field_sig), e.source.clone()),
            e.trans.clone(),
        );
        by_src.insert(e.source.clone(), e.trans.clone());
    }
    move |e: &TmEntry| {
        by.get(&(format!("{}:{}", e.rec_sig, e.field_sig), e.source.clone()))
            .or_else(|| by_src.get(&e.source))
            .cloned()
    }
}

fn stand_in() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 65536];
            let mut at = None;
            loop {
                let n = match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                    let len = head
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
                    at = Some(p + 4);
                    break;
                }
            }
            let Some(at) = at else { continue };
            let v: serde_json::Value =
                serde_json::from_slice(&buf[at..]).unwrap_or(serde_json::Value::Null);
            let user = v["messages"][1]["content"].as_str().unwrap_or("[]");
            let texts: Vec<String> = serde_json::from_str(user).unwrap_or_default();
            let answers: Vec<String> = texts.iter().map(|t| format!("[FR] {t}")).collect();
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
#[ignore]
fn recette() {
    let (Ok(root), Ok(work)) = (
        std::env::var("XIMOD_RECETTE_D06"),
        std::env::var("XIMOD_D06_WORK"),
    ) else {
        eprintln!("XIMOD_RECETTE_D06 / XIMOD_D06_WORK not set: skipped");
        return;
    };
    let root = PathBuf::from(root);
    let work = PathBuf::from(work);
    let _ = std::fs::remove_dir_all(&work);
    let cfg = work.join("cfg");
    std::fs::create_dir_all(cfg.join("data/bases/jeux/SkyrimSE")).unwrap();
    std::fs::create_dir_all(cfg.join("data/bases/mods/SkyrimSE")).unwrap();
    std::fs::copy(
        root.join("Bases/SkyrimSE_ENG-FRA.ximdb"),
        cfg.join("data/bases/jeux/SkyrimSE/SkyrimSE_ENG-FRA.ximdb"),
    )
    .unwrap();
    std::fs::write(
        cfg.join("Config.ini"),
        "[General]\nLocale=fra\n[Translation]\nSourceLanguage=eng\nTargetLanguage=fra\nDefaultGame=SkyrimSE\n",
    )
    .unwrap();
    ximod_translator::config::paths::init_read_only(Some(&cfg));
    let cfg_before = std::fs::read(cfg.join("Config.ini")).unwrap();
    let r = R {
        cfg: cfg.clone(),
        work: work.clone(),
    };
    let mods = work.join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let apoc_dir = root.join("Lot 4-2/Apocalypse - Magic of Skyrim");
    let apoc = mods.join("Apocalypse - Magic of Skyrim.esp");
    std::fs::copy(apoc_dir.join("Apocalypse - Magic of Skyrim.esp"), &apoc).unwrap();
    let caco_dir = root.join("Lot 4-2/Complete Alchemy and Cooking Overhaul");
    let caco = mods.join("complete alchemy & cooking overhaul.esp");
    std::fs::copy(
        caco_dir.join("complete alchemy & cooking overhaul.esp"),
        &caco,
    )
    .unwrap();
    let iw_dir = root.join("Lot 4-1/Immersive Weapons");
    let iw = mods.join("Immersive Weapons.esp");
    std::fs::copy(iw_dir.join("Immersive Weapons.esp"), &iw).unwrap();

    // ------------------------------------------- the translator's work
    let fr_plugin = plugin(&apoc_dir.join(
        "(FR - DSD) Apocalypse - Magic of Skyrim 9.45-4771-9-45-1748024240/08 Clean patch/Apocalypse - Magic of Skyrim.esp",
    ));
    let fr: HashMap<Key, String> = fr_plugin
        .entries
        .iter()
        .map(|e| (key(e), e.source.clone()))
        .collect();
    let n_apoc = mod_dict(&r, &apoc, &|e| fr.get(&key(e)).cloned());
    let n_caco = mod_dict(&r, &caco, &dsd_of(&caco_dir.join(
        "Complete Alchemy and Cooking Overhaul (French) 149068 3.0FRDSD 2026-07-13T12-55Z bqv3tSkvi/Data/SKSE/Plugins/DynamicStringDistributor/complete alchemy & cooking overhaul.esp/complete alchemy & cooking overhaul.json",
    )));
    let iw_dsd = std::fs::read_dir(iw_dir.join(
        "Immersive Weapons - French-93449-2-0FRDSD01-1734781267/Data/SKSE/Plugins/DynamicStringDistributor/immersive weapons.esp",
    ))
    .unwrap()
    .flatten()
    .map(|e| e.path())
    .find(|p| p.extension().is_some_and(|x| x == "json"))
    .unwrap();
    let n_iw = mod_dict(&r, &iw, &dsd_of(&iw_dsd));
    eprintln!("mod dictionaries: Apocalypse {n_apoc}, CACO {n_caco}, IW {n_iw}");

    // ------------------------------------------------- T-D06-01: the script
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/cli/rebuild.sh");
    let out = work.join("out/Apocalypse");
    let t0 = std::time::Instant::now();
    let o = Command::new("bash")
        .arg(&script)
        .arg(&apoc)
        .arg(&out)
        .arg("Apocalypse - Magic of Skyrim")
        .arg("9.45")
        .arg("Patrice")
        .env("XIMOD_CLI", CLI)
        .env("XIMOD_ARGS", format!("--config {}", cfg.display()))
        .output()
        .unwrap();
    eprintln!(
        "rebuild.sh: code {:?} in {:.1} s\n{}{}",
        o.status.code(),
        t0.elapsed().as_secs_f32(),
        text(&o.stdout),
        text(&o.stderr)
    );
    assert_eq!(o.status.code(), Some(0));
    let dsd = out.join("Apocalypse - Magic of Skyrim.json");
    let patch = out.join("Apocalypse - Magic of Skyrim_patch.esp");
    assert!(dsd.is_file() && patch.is_file());
    let m = ximod_translator::manifest::Manifest::parse(
        &std::fs::read_to_string(out.join("manifest.json")).unwrap(),
    )
    .unwrap();
    eprintln!(
        "manifest: source {} {} bytes; outputs {:?}",
        m.inputs[0].crc32,
        m.inputs[0].size,
        m.outputs
            .iter()
            .map(|o| (o.path.clone(), o.lines))
            .collect::<Vec<_>>()
    );
    let zip = work.join("out/Apocalypse.zip");
    let z = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
    let names: Vec<String> = z.file_names().map(str::to_string).collect();
    eprintln!("package: {names:?}");
    assert!(names.iter().any(|n| {
        n.starts_with("SKSE/Plugins/DynamicStringDistributor/Apocalypse - Magic of Skyrim.esp/")
    }));
    assert!(names.contains(&"fomod/manifest.json".to_string()));
    // The same script on a plugin that is not there: 2, nothing written.
    let o = Command::new("bash")
        .arg(&script)
        .arg(mods.join("absent.esp"))
        .arg(work.join("out/absent"))
        .env("XIMOD_CLI", CLI)
        .env("XIMOD_ARGS", format!("--config {}", cfg.display()))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!work.join("out/absent").exists() && !work.join("out/absent.zip").exists());

    // Identity: the DSD of the command line is the one the library writes
    // from the mod's dictionary (the export of the interface).
    let dict = r.mods().join("Apocalypse - Magic of Skyrim_ENG-FRA.ximdb");
    let tm = {
        let cfg = ximod_translator::config::AppConfig::load_read_only();
        let formats = ximod_translator::textfmt::Formats::load();
        let mut o = ximod_translator::headless::read(
            &apoc,
            ximod_translator::headless::Kind::Plugin,
            "SkyrimSE",
            "eng",
            "fra",
            &cfg,
            &formats,
        )
        .unwrap();
        ximod_translator::headless::remember(&cfg, &mut o);
        let db = ximod_translator::basedb::FileDbCache::default()
            .get(&dict)
            .unwrap();
        let only = ximod_translator::dict_stack::DictStack {
            levels: vec![ximod_translator::dict_stack::Level {
                role: ximod_translator::dicts::Role::Mod,
                path: dict.clone(),
                db,
            }],
            normalize: false,
        };
        let plan = ximod_translator::headless::rules_plan(&cfg, &o);
        ximod_translator::headless::apply(&mut o, Some(&only), &plan);
        o.tm
    };
    let policy =
        ximod_translator::export::ExportPolicy::validated_only().with(Status::Pending, true);
    let (bytes, n) = ximod_translator::tm::dsd::to_json_bytes(&policy.filtered(&tm));
    assert_eq!(std::fs::read(&dsd).unwrap(), bytes, "DSD identical");
    eprintln!("DSD identical to the library's: {n} lines");

    // ------------------------------- T-D06-03: the assistants (B-21)
    // The spells and effects translated, the scrolls and tomes not.
    // Its own configuration: only the game's base and its own mod's
    // dictionary in the stack (the full one would fill the scrolls).
    let cfg2 = work.join("cfg2");
    let mods2 = cfg2.join("mods").join("SkyrimSE");
    std::fs::create_dir_all(&mods2).unwrap();
    std::fs::write(
        cfg2.join("Config.ini"),
        format!(
            "[General]\nLocale=fra\n[Translation]\nSourceLanguage=eng\nTargetLanguage=fra\nDefaultGame=SkyrimSE\n[Paths]\nGameBasesDir={}\nModBasesDir={}\n",
            cfg.join("data/bases/jeux").display(),
            cfg2.join("mods").display()
        ),
    )
    .unwrap();
    let apoc2 = work.join("b21").join("Apocalypse - Magic of Skyrim.esp");
    std::fs::create_dir_all(apoc2.parent().unwrap()).unwrap();
    std::fs::copy(&apoc, &apoc2).unwrap();
    let spells = |e: &TmEntry| {
        matches!(e.rec_sig.as_str(), "SPEL" | "MGEF")
            .then(|| fr.get(&key(e)).cloned())
            .flatten()
    };
    mod_dict_in(&mods2, &apoc2, &spells);
    let o = Command::new(CLI)
        .arg("--config")
        .arg(&cfg2)
        .args([
            "--json",
            "assist",
            apoc2.to_str().unwrap(),
            "--derive",
            "--patterns",
            "--glossary",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let j = &json(&o)[0];
    eprintln!(
        "assist: filled {} (composed {}, glossary {}, patterns {}, derived {})",
        j["filled"], j["composed"], j["glossary"], j["patterns"], j["derived"]
    );
    // The library, as the dialog: the same plan.
    {
        let mut cfg = ximod_translator::config::AppConfig::load_read_only();
        cfg.game_bases_dir = work.join("cfg/data/bases/jeux").display().to_string();
        cfg.mod_bases_dir = cfg2.join("mods").display().to_string();
        let formats = ximod_translator::textfmt::Formats::load();
        let mut o2 = ximod_translator::headless::read(
            &apoc2,
            ximod_translator::headless::Kind::Plugin,
            "SkyrimSE",
            "eng",
            "fra",
            &cfg,
            &formats,
        )
        .unwrap();
        ximod_translator::headless::remember(&cfg, &mut o2);
        let mut cache = ximod_translator::basedb::FileDbCache::default();
        let stack = ximod_translator::headless::stack(&cfg, &apoc2, &o2.settings, &mut cache);
        eprintln!(
            "  stack: {:?}",
            stack
                .as_ref()
                .map(|s| s.levels.iter().map(|l| l.name()).collect::<Vec<_>>())
        );
        let plan = ximod_translator::headless::rules_plan(&cfg, &o2);
        ximod_translator::headless::apply(&mut o2, stack.as_ref(), &plan);
        let a = ximod_translator::assist::Assistant::load("ENG-FRA", "SkyrimSE");
        let bases = ximod_translator::assist::StackBases(
            stack
                .as_ref()
                .map(|s| s.levels.iter().map(|l| l.db.clone()).collect())
                .unwrap_or_default(),
        );
        let teaches = ximod_translator::assist::derive::teaches(&apoc2);
        let ctx = a
            .derive
            .as_deref()
            .map(|f| ximod_translator::assist::DeriveCtx {
                file: f,
                lang: "fra",
                open: Vec::new(),
                parent_bases: &bases,
                teaches: &teaches,
            });
        let lines: Vec<usize> = (0..o2.tm.entries.len()).collect();
        let p = ximod_translator::assist::plan_all(
            &o2.tm.entries,
            &lines,
            &a,
            &ximod_translator::assist::Options::default(),
            &bases,
            ctx.as_ref(),
        );
        eprintln!(
            "  the dialog's plan: {} (derived {:?}, parents untranslated {}, not found {})",
            p.filled(),
            p.derived,
            p.untranslated,
            p.not_found
        );
        assert_eq!(j["filled"].as_u64().unwrap() as usize, p.filled());
    }
    // Written: the lines En attente in the mod's dictionary, with the
    // provenance of their rule.
    let o = Command::new(CLI)
        .arg("--config")
        .arg(&cfg2)
        .args(["assist", apoc2.to_str().unwrap(), "--write-dict"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    let d = ximod_translator::basedb::ximdb_to_tm(
        &mods2.join("Apocalypse - Magic of Skyrim_ENG-FRA.ximdb"),
    )
    .unwrap();
    let rule_lines = d
        .entries
        .iter()
        .filter(|e| e.prov.starts_with("rule:") && status::derive(e) == Status::Pending)
        .count();
    eprintln!("  mod's dictionary: {rule_lines} lines of the assistants, En attente");
    let _ = (&r, Path::new(""));

    // ------------------------- T-D06-03: translate with a stand-in service
    let pdir = cfg.join("data/online/profiles");
    std::fs::create_dir_all(&pdir).unwrap();
    let shipped =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/online/profiles/local.json");
    let mut prof: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(shipped).unwrap()).unwrap();
    prof["id"] = "mock".into();
    prof["name"] = "Mock".into();
    prof["endpoint"] = stand_in().into();
    prof["model"] = "test".into();
    prof["maxRpm"] = 6000.into();
    std::fs::write(pdir.join("mock.json"), prof.to_string()).unwrap();
    let o = r.cli(&[
        "translate",
        iw.to_str().unwrap(),
        "--service",
        "openai:mock",
    ]);
    assert_eq!(o.status.code(), Some(2));
    eprintln!("translate without --max-chars: {}", text(&o.stderr).trim());
    let o = r.cli(&[
        "--json",
        "--verbose",
        "translate",
        iw.to_str().unwrap(),
        "--service",
        "openai:mock",
        "--max-chars",
        "1000000",
    ]);
    let j = &json(&o)[0];
    eprintln!(
        "translate IW: code {:?}, lines {}, distinct {}, chars {}, received {}, refused {}, skipped {}",
        o.status.code(),
        j["lines"],
        j["items"],
        j["chars"],
        j["received"],
        j["refused"],
        j["skipped"]
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));

    // ------------------------------------------- T-D06-04: the batch
    let batch = |output: &str, stop: bool| {
        format!(
            r#"{{ "format": 1, "game": "SkyrimSE", "from": "eng", "to": "fra", "out": "lot",
                 "stopOnError": {stop},
                 "jobs": [ {{ "files": "mods/*.esp",
                   "steps": [ "apply", "check",
                     {{ "export": {{ "output": "{output}", "statuses": ["validated", "pending"] }} }},
                     "manifest" ] }} ] }}"#
        )
    };
    let lot = work.join("lot.json");
    std::fs::write(&lot, batch("dsdx", false)).unwrap();
    let o = r.cli(&["batch", lot.to_str().unwrap()]);
    eprintln!(
        "batch (export dsdx): code {:?} — {}",
        o.status.code(),
        text(&o.stderr).trim()
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(!work.join("lot").exists());
    std::fs::write(&lot, batch("dsd", false)).unwrap();
    let t0 = std::time::Instant::now();
    let o = r.cli(&["batch", lot.to_str().unwrap()]);
    eprintln!(
        "batch: code {:?} in {:.1} s\n{}{}",
        o.status.code(),
        t0.elapsed().as_secs_f32(),
        text(&o.stdout),
        text(&o.stderr)
    );
    assert!(o.status.code().unwrap() <= 1);
    // A plugin whose translation lost its numbers: check in error, code 1,
    // the batch goes on.
    let bad = mods.join("zz broken.esp");
    std::fs::copy(&iw, &bad).unwrap();
    mod_dict(&r, &bad, &|e| {
        Some(format!(
            "FR {}",
            e.source.replace(|c: char| c.is_ascii_digit(), "")
        ))
    });
    let o = r.cli(&["--json", "batch", lot.to_str().unwrap()]);
    let j = json(&o);
    let end = j.last().unwrap();
    eprintln!(
        "batch with a broken translation: code {:?}, end {end}",
        o.status.code()
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(
        work.join("lot/zz broken/manifest.json").is_file(),
        "went on"
    );
    // stopOnError: stops at the first code 2 (a step that cannot be done).
    std::fs::write(&lot, batch("hybrid", true)).unwrap();
    let o = r.cli(&["--json", "batch", lot.to_str().unwrap()]);
    let end = json(&o).last().unwrap().clone();
    eprintln!(
        "batch hybrid + stopOnError: code {:?}, end {end}",
        o.status.code()
    );

    // ------------------------------------------------- T-D06-07
    assert_eq!(std::fs::read(cfg.join("Config.ini")).unwrap(), cfg_before);
    assert!(!cfg.join("autosave").exists() && !cfg.join("logs").exists());
    eprintln!("Config.ini unchanged, no autosave, no log");
    let _ = &r.work;
}
