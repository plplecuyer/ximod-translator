//! D-01 / D-02 end to end against a local stand-in of the services: a tiny
//! HTTP server on a loopback port answers like DeepL (`/v2/translate`,
//! `/v2/usage`), Azure (`/translate`) and an OpenAI-compatible server
//! (`/v1/chat/completions`, `/v1/models`), translating by prefixing « FR: »
//! and keeping the protection tokens — or dropping one when the text says
//! « LOSE ».

use std::io::{Read, Write};
use std::net::TcpListener;

use ximod_translator::online::job::{JobInput, JobMsg};
use ximod_translator::online::quota::Item;
use ximod_translator::online::{Error, OnlineConfig, Service};

fn fake_translate(text: &str) -> String {
    let mut t = format!("FR: {text}");
    if text.contains("LOSE") {
        t = t.replacen("<a_A>", "", 1);
    }
    t
}

/// Start the stand-in; returns its base URL.
fn start_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            // Read the head, then the body by Content-Length.
            let mut head_end = None;
            loop {
                let n = match s.read(&mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buf.extend_from_slice(&tmp[..n]);
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    head_end = Some(p + 4);
                    let head = String::from_utf8_lossy(&buf[..p]).to_string();
                    let len = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
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
            let first = head.lines().next().unwrap_or("").to_string();
            let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
            let auth_ok = head.to_lowercase().contains("deepl-auth-key good")
                || head
                    .to_lowercase()
                    .contains("ocp-apim-subscription-key: good")
                || !path.starts_with("/v2") && !path.starts_with("/translate");
            let (status, out) = if !auth_ok {
                (
                    "403 Forbidden",
                    r#"{"message":"Authorization failed. Please supply a valid auth key."}"#
                        .to_string(),
                )
            } else if path.starts_with("/v2/usage") {
                (
                    "200 OK",
                    r#"{"character_count":36540,"character_limit":500000}"#.to_string(),
                )
            } else if path.starts_with("/v2/translate") {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                let texts: Vec<String> = v["text"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t.as_str().unwrap().to_string())
                    .collect();
                let tr: Vec<serde_json::Value> = texts
                    .iter()
                    .map(|t| serde_json::json!({"detected_source_language":"EN","text":fake_translate(t)}))
                    .collect();
                (
                    "200 OK",
                    serde_json::json!({"translations": tr}).to_string(),
                )
            } else if path.starts_with("/translate") {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                let arr: Vec<serde_json::Value> = v
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| serde_json::json!({"translations":[{"text":fake_translate(t["Text"].as_str().unwrap()),"to":"fr"}]}))
                    .collect();
                ("200 OK", serde_json::Value::Array(arr).to_string())
            } else if path.starts_with("/v1/models") {
                ("200 OK", r#"{"data":[{"id":"fake-model"}]}"#.to_string())
            } else if path.starts_with("/v1/chat/completions") {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap();
                let user = v["messages"][1]["content"].as_str().unwrap();
                let texts: Vec<String> = serde_json::from_str(user).unwrap();
                let answers: Vec<String> = texts.iter().map(|t| fake_translate(t)).collect();
                let content = serde_json::to_string(&answers).unwrap();
                ("200 OK", serde_json::json!({"choices":[{"message":{"role":"assistant","content":content}}]}).to_string())
            } else {
                (
                    "404 Not Found",
                    r#"{"message":"no such route"}"#.to_string(),
                )
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                out.len()
            );
            let _ = s.write_all(resp.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// T-D01-01 / T-D02-01: DeepL stand-in — usage at the test, tokens kept,
/// a lost token refused by the restoration, a bad key reported with the
/// service's message.
#[test]
fn deepl_round_trip_with_protection() {
    let base = start_server();
    let cfg = OnlineConfig {
        deepl_endpoint: base.clone(),
        max_rpm: 600,
        ..Default::default()
    };
    let c = ximod_translator::online::connector(&Service::DeepL);
    assert_eq!(c.test(&cfg, "good").unwrap(), "usage:36540/500000");
    match c.test(&cfg, "bad") {
        Err(Error::Http { code: 403, message }) => {
            assert!(message.contains("Authorization failed"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(c.test(&cfg, ""), Err(Error::NoKey));
    let set = ximod_translator::tags::TagSet::from_names(&["Alias*"]);
    let src = "Give <Alias=Victim> 3 gold.\nNow.";
    let (sent, tokens) = ximod_translator::online::protect::protect(
        src,
        &set,
        ximod_translator::online::protect::Options::all(),
    );
    let out = c
        .translate(&cfg, "good", std::slice::from_ref(&sent), "en", "fr")
        .unwrap();
    assert_eq!(out[0], format!("FR: {sent}"));
    let restored = ximod_translator::online::protect::restore(&out[0], &tokens, src).unwrap();
    assert_eq!(restored, "FR: Give <Alias=Victim> 3 gold.\nNow.");
    // A lost token.
    let (sent2, tokens2) = ximod_translator::online::protect::protect(
        "LOSE <Alias=A> it",
        &set,
        ximod_translator::online::protect::Options::all(),
    );
    let out2 = c.translate(&cfg, "good", &[sent2], "en", "fr").unwrap();
    assert!(
        ximod_translator::online::protect::restore(&out2[0], &tokens2, "LOSE <Alias=A> it")
            .is_err()
    );
}

/// T-D01-03: a job of 184 lines in 4 requests, then served by the cache.
#[test]
fn job_batches_and_caches() {
    let base = start_server();
    let cfg = OnlineConfig {
        deepl_endpoint: base,
        max_rpm: 6000,
        ..Default::default()
    };
    let dir = std::env::temp_dir().join(format!("ximod_online_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cache = dir.join("deepl_EN-FR.ximdb");
    let items: Vec<Item> = (0..184)
        .map(|k| Item {
            key: k,
            text: format!("Line number {k} of the mod."),
        })
        .collect();
    let input = JobInput {
        service: Service::DeepL,
        cfg: cfg.clone(),
        key: "good".into(),
        src: "en".into(),
        dst: "fr".into(),
        items: items.clone(),
        cache_path: Some(cache.clone()),
    };
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let rx = ximod_translator::online::job::start(input, stop.clone());
    let msgs: Vec<JobMsg> = rx.iter().collect();
    let answers = msgs
        .iter()
        .filter(|m| {
            matches!(
                m,
                JobMsg::Answer {
                    result: Ok(_),
                    cached: false,
                    ..
                }
            )
        })
        .count();
    assert_eq!(answers, 184);
    let total = msgs
        .iter()
        .find_map(|m| match m {
            JobMsg::Progress { total, .. } => Some(*total),
            _ => None,
        })
        .unwrap();
    assert_eq!(total, 4);
    match msgs.last() {
        Some(JobMsg::Done {
            stopped: false,
            sent_chars,
        }) => assert!(*sent_chars > 184 * 20),
        other => panic!("{other:?}"),
    }
    // Again: everything from the cache, nothing sent.
    let input = JobInput {
        service: Service::DeepL,
        cfg,
        key: "good".into(),
        src: "en".into(),
        dst: "fr".into(),
        items,
        cache_path: Some(cache),
    };
    let rx = ximod_translator::online::job::start(input, stop);
    let msgs: Vec<JobMsg> = rx.iter().collect();
    let cached = msgs
        .iter()
        .filter(|m| matches!(m, JobMsg::Answer { cached: true, .. }))
        .count();
    assert_eq!(cached, 184);
    assert!(matches!(
        msgs.last(),
        Some(JobMsg::Done { sent_chars: 0, .. })
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// T-D01-05: Azure and an OpenAI-compatible server without a key.
#[test]
fn azure_and_openai_stand_ins() {
    let base = start_server();
    let cfg = OnlineConfig {
        azure_endpoint: base.clone(),
        ..Default::default()
    };
    let az = ximod_translator::online::connector(&Service::Azure);
    assert_eq!(
        az.translate(&cfg, "good", &["Hello <a_A>".into()], "en", "fr")
            .unwrap(),
        vec!["FR: Hello <a_A>"]
    );
    assert!(az.test(&cfg, "good").unwrap().starts_with("control:FR: "));
    let oa =
        ximod_translator::online::openai::OpenAI::new(ximod_translator::online::profile::Profile {
            id: "local".into(),
            name: "Local".into(),
            endpoint: format!("{base}/v1"),
            model: "fake-model".into(),
            prompt: "From %lang_source% to %lang_dest%.".into(),
            key_optional: true,
            ..Default::default()
        });
    use ximod_translator::online::Connector;
    assert_eq!(
        oa.translate(&cfg, "", &["One".into(), "Two <n_A>".into()], "en", "fr")
            .unwrap(),
        vec!["FR: One", "FR: Two <n_A>"]
    );
    let t = oa.test(&cfg, "").unwrap();
    assert!(t.starts_with("model:fake-model\u{1f}known\u{1f}"), "{t}");
}
