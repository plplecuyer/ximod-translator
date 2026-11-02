//! D-03: the profiles of the OpenAI-compatible connections, end to end
//! against a local stand-in that records the requests: the prompt and its
//! variables, the parameters, the « lines » form, an incomplete answer sent
//! again line by line, the profile's guards, validation and the files.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use ximod_translator::online::job::{JobInput, JobMsg};
use ximod_translator::online::profile::{self, Answer, Invalid, Origin, Profile, Warning};
use ximod_translator::online::quota::Item;
use ximod_translator::online::{Connector, Error, OnlineConfig, Service};

type Seen = Arc<Mutex<Vec<serde_json::Value>>>;

/// The profiles of the session are global: the tests that set them run
/// one at a time.
static SESSION: Mutex<()> = Mutex::new(());

fn fake(t: &str) -> String {
    format!("FR: {t}")
}

/// The stand-in: `/v1/models` and `/v1/chat/completions`, JSON or lines
/// (« ⟨⟨n⟩⟩ » markers); a text holding « DROP » makes a batch answer one
/// text short. Every request body is kept.
fn start_server(seen: Seen) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let re = regex::Regex::new(r"⟨⟨(\d+)⟩⟩\n").unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            let mut head_end = None;
            loop {
                let n = match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    head_end = Some(p + 4);
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
                    break;
                }
            }
            let Some(he) = head_end else { continue };
            let head = String::from_utf8_lossy(&buf[..he]).to_string();
            let body = String::from_utf8_lossy(&buf[he..]).to_string();
            let path = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("")
                .to_string();
            let out = if path.starts_with("/v1/models") {
                r#"{"data":[{"id":"mistral-nemo:latest"}]}"#.to_string()
            } else {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                let mut kept = v.clone();
                kept["_auth"] = serde_json::json!(head.to_lowercase().contains("authorization:"));
                kept["_x"] = serde_json::json!(head.to_lowercase().contains("x-test: yes"));
                seen.lock().unwrap().push(kept);
                let user = v["messages"][1]["content"].as_str().unwrap().to_string();
                let content = if let Ok(texts) = serde_json::from_str::<Vec<String>>(&user) {
                    let mut a: Vec<String> = texts.iter().map(|t| fake(t)).collect();
                    if texts.len() > 1 && texts.iter().any(|t| t.contains("DROP")) {
                        a.pop();
                    }
                    serde_json::to_string(&a).unwrap()
                } else {
                    // Lines: « ⟨⟨n⟩⟩\ntext » blocks, answered in reverse order.
                    let parts: Vec<&str> = re.split(&user).skip(1).collect();
                    let mut blocks: Vec<String> = parts
                        .iter()
                        .enumerate()
                        .map(|(i, t)| format!("⟨⟨{}⟩⟩ {}", i + 1, fake(t.trim())))
                        .collect();
                    if parts.len() > 1 && user.contains("DROP") {
                        blocks.pop();
                    }
                    blocks.reverse();
                    blocks.join("\n")
                };
                serde_json::json!({"choices":[{"message":{"role":"assistant","content":content}}]})
                    .to_string()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                out.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/v1")
}

fn shipped_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/online/profiles")
}

/// The shipped « Ollama local » profile pointed at the stand-in.
fn local(endpoint: &str) -> Profile {
    let mut p = profile::shipped(&shipped_dir(), "local").unwrap();
    p.endpoint = endpoint.to_string();
    p.model = "mistral-nemo:latest".into();
    p
}

fn run_job(service: Service, texts: &[&str], cfg: &OnlineConfig) -> Vec<JobMsg> {
    let input = JobInput {
        service: service.clone(),
        cfg: cfg.for_service(&service),
        key: String::new(),
        src: "en".into(),
        dst: "fr".into(),
        items: texts
            .iter()
            .enumerate()
            .map(|(key, t)| Item {
                key,
                text: t.to_string(),
            })
            .collect(),
        cache_path: None,
    };
    ximod_translator::online::job::start(input, Arc::new(AtomicBool::new(false)))
        .iter()
        .collect()
}

fn answers(msgs: &[JobMsg]) -> Vec<(usize, Result<String, Error>)> {
    let mut v: Vec<(usize, Result<String, Error>)> = msgs
        .iter()
        .filter_map(|m| match m {
            JobMsg::Answer { key, result, .. } => Some((*key, result.clone())),
            _ => None,
        })
        .collect();
    v.sort_by_key(|a| a.0);
    v
}

#[test]
fn a_local_server_without_a_key() {
    let _g = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    // T-D03-01 (critère, against the stand-in; the real Ollama at the
    // sponsor's): the shipped profile, its model filled in, no key.
    let seen: Seen = Arc::default();
    let base = start_server(seen.clone());
    let mut p = local(&base);
    p.headers.insert("X-Test".into(), "yes".into());
    p.prompt = "Translate from %lang_source% to %lang_dest% for %game% (%n% texts).".into();
    profile::set_all(vec![p.clone()]);
    let s = Service::OpenAI("local".into());
    assert!(s.key_optional());
    assert_eq!(s.id(), "openai:local");
    assert_eq!(s.profile_name().as_deref(), Some("Ollama local"));
    // Tester…: the model is in the list (its name holds « : »).
    let c = ximod_translator::online::openai::OpenAI::new(p.clone());
    let t = c.test(&OnlineConfig::default(), "").unwrap();
    assert_eq!(
        t,
        "model:mistral-nemo:latest\u{1f}known\u{1f}FR: The dragon is coming."
    );
    let cfg = OnlineConfig {
        game: "The Elder Scrolls V: Skyrim Special Edition".into(),
        ..Default::default()
    };
    let msgs = run_job(s, &["Hello", "Iron Sword <n_A>"], &cfg);
    let a = answers(&msgs);
    assert_eq!(a[0].1, Ok("FR: Hello".into()));
    assert_eq!(a[1].1, Ok("FR: Iron Sword <n_A>".into()));
    // T-D03-02: the variables in English, the game, the count; the
    // parameters of the profile, its header, no key sent.
    let req = seen.lock().unwrap().last().cloned().unwrap();
    let system = req["messages"][0]["content"].as_str().unwrap();
    assert_eq!(
        system,
        "Translate from English to French for The Elder Scrolls V: Skyrim Special Edition (2 texts)."
    );
    assert_eq!(req["model"], "mistral-nemo:latest");
    assert_eq!(req["temperature"], 0.2);
    assert!(req.get("frequency_penalty").is_none());
    assert_eq!(req["_auth"], false);
    assert_eq!(req["_x"], true);
}

#[test]
fn an_eet_prompt_and_the_lines_form() {
    let _g = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    // T-D03-02 / 03.
    let seen: Seen = Arc::default();
    let base = start_server(seen.clone());
    let mut p = local(&base);
    p.id = "lines".into();
    p.prompt = "Traduis de %lang_source% vers %lang_dest%.".into();
    p.answer = Answer::Lines;
    p.max_lines = Some(20);
    p.max_tokens = Some(2048);
    p.max_rpm = Some(600);
    p.frequency_penalty = Some(0.5);
    profile::set_all(vec![p]);
    let s = Service::OpenAI("lines".into());
    let texts: Vec<String> = (1..=20).map(|i| format!("Text {i}")).collect();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let msgs = run_job(s.clone(), &refs, &OnlineConfig::default());
    let a = answers(&msgs);
    assert_eq!(a.len(), 20);
    // Read back in the order sent, although answered in reverse.
    assert!(
        a.iter()
            .all(|(k, r)| *r == Ok(format!("FR: Text {}", k + 1)))
    );
    let req = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(
        req["messages"][0]["content"],
        "Traduis de English vers French."
    );
    assert_eq!(req["max_tokens"], 2048);
    assert_eq!(req["frequency_penalty"], 0.5);
    // One text short: the batch is sent again line by line.
    seen.lock().unwrap().clear();
    let mut texts = texts.clone();
    texts[3] = "Text 4 DROP".into();
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let msgs = run_job(s, &refs, &OnlineConfig::default());
    let a = answers(&msgs);
    assert_eq!(a.iter().filter(|(_, r)| r.is_ok()).count(), 20);
    assert_eq!(seen.lock().unwrap().len(), 21, "one batch, then 20 lines");
}

#[test]
fn guards_and_caches_per_profile() {
    let _g = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    // T-D03-04.
    let mut a = local("http://localhost:11434/v1");
    a.id = "a".into();
    a.max_lines = Some(20);
    let mut b = a.clone();
    b.id = "b".into();
    b.max_lines = Some(50);
    b.max_chars = None;
    profile::set_all(vec![a, b]);
    let common = OnlineConfig::default();
    let (sa, sb) = (Service::OpenAI("a".into()), Service::OpenAI("b".into()));
    assert_eq!(common.for_service(&sa).max_lines, 20);
    assert_eq!(common.for_service(&sb).max_lines, 50);
    assert_eq!(common.for_service(&sb).max_chars, common.max_chars);
    assert_eq!(
        common.for_service(&Service::DeepL).max_lines,
        common.max_lines
    );
    let items: Vec<Item> = (0..100)
        .map(|key| Item {
            key,
            text: format!("Line {key}"),
        })
        .collect();
    let plan = |s: &Service| {
        let c = common.for_service(s);
        ximod_translator::online::quota::plan(items.clone(), c.max_chars, c.max_lines)
            .batches
            .len()
    };
    assert_eq!((plan(&sa), plan(&sb)), (5, 2));
    let (pa, pb) = (
        ximod_translator::online::cache::Cache::path_for(&sa, "en", "fr"),
        ximod_translator::online::cache::Cache::path_for(&sb, "en", "fr"),
    );
    assert_ne!(pa, pb);
    assert!(pa.ends_with("openai-a_EN-FR.ximdb"));
    // The « openai » profile keeps the cache of the first version.
    assert!(
        ximod_translator::online::cache::Cache::path_for(
            &Service::OpenAI("openai".into()),
            "en",
            "fr"
        )
        .ends_with("openai_EN-FR.ximdb")
    );
}

#[test]
fn validation_files_and_the_first_version() {
    // T-D03-05 / 06.
    let shipped = shipped_dir();
    for id in ["openai", "mistral"] {
        let p = profile::shipped(&shipped, id).unwrap();
        assert_eq!(p.validate(), Ok(vec![]), "{id}");
    }
    // The local profile is shipped without a model: to be filled in.
    assert_eq!(
        profile::shipped(&shipped, "local").unwrap().validate(),
        Err(Invalid::Model)
    );
    let good = local("http://localhost:11434/v1");
    let mut bad = good.clone();
    bad.temperature = Some(3.0);
    assert!(matches!(
        bad.validate(),
        Err(Invalid::Range {
            field: "temperature",
            ..
        })
    ));
    let mut bad = good.clone();
    bad.endpoint = String::new();
    assert_eq!(bad.validate(), Err(Invalid::Endpoint));
    let mut warn = good.clone();
    warn.prompt = "Translate.".into();
    assert_eq!(warn.validate(), Ok(vec![Warning::NoTarget]));
    // The user's folder: an invalid profile is not written, a copy wins,
    // removing it brings the shipped one back, a new one comes after.
    let dir = std::env::temp_dir().join(format!("ximod-profiles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut b = good.clone();
    b.temperature = Some(9.0);
    assert!(profile::save(&dir, &b).is_err());
    assert!(!dir.join("local.json").exists());
    profile::save(&dir, &good).unwrap();
    let mut own = good.clone();
    own.id = profile::new_id("Mon serveur", &["local".into()]);
    own.name = "Mon serveur".into();
    profile::save(&dir, &own).unwrap();
    let all = profile::load_all(&shipped, Some(&dir));
    let ids: Vec<&str> = all.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["openai", "local", "mistral", "mon-serveur"]);
    assert_eq!(all[1].origin, Origin::UserCopy);
    assert_eq!(all[1].model, "mistral-nemo:latest");
    assert_eq!(all[3].origin, Origin::User);
    // The key is never in the file.
    let text = std::fs::read_to_string(dir.join("local.json")).unwrap();
    assert!(!text.to_lowercase().contains("key\""), "{text}");
    profile::remove(&dir, "local").unwrap();
    let all = profile::load_all(&shipped, Some(&dir));
    assert_eq!(all[1].origin, Origin::Shipped);
    assert_eq!(all[1].model, "");
    let _ = std::fs::remove_dir_all(&dir);
    // The first version's « openai » is the « openai » profile.
    assert_eq!(
        Service::from_id("openai"),
        Some(Service::OpenAI("openai".into()))
    );
    assert_eq!(Service::OpenAI("openai".into()).id(), "openai:openai");
    assert_eq!(
        Service::from_id("openai:local"),
        Some(Service::OpenAI("local".into()))
    );
    assert_eq!(Service::from_id("openai:../x"), None);
}
