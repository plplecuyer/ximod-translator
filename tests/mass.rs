//! Traduction de masse (D-04, T-D04-01 / 02): the whole document through
//! a stand-in OpenAI-compatible server, stopped after a number of
//! requests: the lines received are En attente with the provenance, none
//! Validé, the others untouched, the scope's account explained, and one
//! undo takes the job back.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ximod_translator::games::Game;
use ximod_translator::online::job::JobInput;
use ximod_translator::online::mass::{self, Outcome, Reason, Skipped};
use ximod_translator::online::protect;
use ximod_translator::online::{OnlineConfig, Service, profile};
use ximod_translator::status::{self, Status};
use ximod_translator::tm::TmEntry;
use ximod_translator::undo::{OpKind, UndoStack, record};

/// The stand-in: `/v1/chat/completions` answering « FR: text » for each
/// text of the JSON array, « KEEP » left as it is (the source again). At
/// the `stop_at`-th request, « Arrêter » is pressed before it is answered:
/// the job ends after the request in flight.
fn start_server(requests: Arc<AtomicUsize>, stop: Arc<AtomicBool>, stop_at: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
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
                    body_at = Some(p + 4);
                    break;
                }
            }
            let Some(at) = body_at else { continue };
            if requests.fetch_add(1, Ordering::SeqCst) + 1 == stop_at {
                stop.store(true, Ordering::Relaxed);
            }
            let v: serde_json::Value =
                serde_json::from_slice(&buf[at..]).unwrap_or(serde_json::Value::Null);
            let user = v["messages"][1]["content"].as_str().unwrap_or("[]");
            let texts: Vec<String> = serde_json::from_str(user).unwrap_or_default();
            let answers: Vec<String> = texts
                .iter()
                .map(|t| {
                    if t.contains("KEEP") {
                        t.clone()
                    } else {
                        format!("FR: {t}")
                    }
                })
                .collect();
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

/// A document of 300 lines: 240 to translate (all distinct but one pair),
/// 25 locked or Ignoré, 5 never shown in game, 20 Validé, 10 En attente
/// already translated.
fn document() -> Vec<TmEntry> {
    let mut v = Vec::new();
    for i in 0..300 {
        let mut e = TmEntry {
            source: format!("Spell of {}", protect::letters(i)),
            rec_sig: "SPEL".into(),
            field_sig: "FULL".into(),
            ..Default::default()
        };
        match i {
            240..=264 => status::set(&mut e, Status::Ignored),
            265..=269 => e.unused = true,
            270..=289 => {
                e.trans = format!("Sort {i}");
                status::set(&mut e, Status::Validated);
            }
            290..=299 => {
                e.trans = format!("Sort {i}");
                status::set(&mut e, Status::Pending);
            }
            _ => {}
        }
        v.push(e);
    }
    // A source twice: sent once, posed on both lines.
    v[1].source = v[0].source.clone();
    // An answer equal to the source: refused.
    v[2].source = "KEEP this".into();
    v
}

fn service(endpoint: &str) -> Service {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/data/online/profiles");
    let mut p = profile::shipped(&dir, "local").unwrap();
    p.endpoint = endpoint.to_string();
    p.model = "mistral-nemo:latest".into();
    p.max_lines = Some(10);
    p.max_rpm = Some(600);
    profile::set_all(vec![p]);
    Service::OpenAI("local".into())
}

#[test]
fn the_whole_document_stopped_after_five_requests() {
    let requests = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let s = service(&start_server(requests.clone(), stop.clone(), 5));
    let mut entries = document();
    let before = entries.clone();

    // R-D04-01 / 02: the scope and its account.
    let (lines, skipped) = mass::select(&entries, 0..entries.len(), true);
    assert_eq!(lines.len(), 240);
    assert_eq!(
        skipped,
        Skipped {
            locked: 30,
            done: 30
        }
    );
    // Unticked, the translated lines come back, the Validé ones asked for.
    let (all, sk) = mass::select(&entries, 0..entries.len(), false);
    assert_eq!((all.len(), sk.locked, sk.done), (270, 30, 0));
    assert_eq!(mass::protected_lines(&entries, &all), 20);
    // A view (a filter) is not the document.
    let view: Vec<usize> = (0..300).filter(|i| i % 3 == 0).collect();
    assert_ne!(mass::select(&entries, view, true).0.len(), lines.len());

    let prepared = mass::prepare(&entries, &lines, Game::SkyrimSE, protect::Options::all());
    assert_eq!(prepared.items.len(), 239);
    let input = JobInput {
        service: s.clone(),
        cfg: OnlineConfig::default().for_service(&s),
        key: String::new(),
        src: "en".into(),
        dst: "fr".into(),
        items: prepared.job_items(),
        cache_path: None,
    };
    let rx = ximod_translator::online::job::start(input, stop.clone());
    let mut seen = Vec::new();
    let got = mass::collect(rx, |done, _| seen.push(done));
    assert_eq!(seen, vec![0, 1, 2, 3, 4, 5]);
    assert!(got.stopped);
    assert_eq!(got.done, 5);
    assert_eq!(got.total, 24);
    assert_eq!(requests.load(Ordering::SeqCst), 5);

    // R-D04-03: posed in one operation, En attente, never Validé.
    let outcomes = mass::outcomes(&prepared.items, &got.answers);
    let mut undo = UndoStack::new(50);
    let prov = ximod_translator::provenance::online(&s.id());
    let summary = record(
        &mut undo,
        &mut entries,
        OpKind::Online,
        Some("test".into()),
        |e| mass::apply(e, &prepared.lines, &outcomes, &prov),
    );
    // 50 sources answered = 51 lines (the pair), one refused (« KEEP »).
    assert_eq!(summary.received, 50);
    assert_eq!(summary.refused, 1);
    assert_eq!(summary.by_reason.get(&Reason::Unchanged), Some(&1));
    assert_eq!(summary.left, 240 - 51);
    assert_eq!(
        outcomes.iter().filter(|o| **o == Outcome::Left).count(),
        239 - 50
    );
    for (i, e) in entries.iter().enumerate() {
        let st = status::derive(e);
        if e.prov == prov {
            assert_eq!(st, Status::Pending, "line {i}");
            assert!(e.trans.starts_with("FR: "), "line {i}");
        } else {
            assert_eq!(e.trans, before[i].trans, "line {i} untouched");
            assert_eq!(st, status::derive(&before[i]), "line {i} untouched");
        }
    }
    assert_eq!(
        entries
            .iter()
            .filter(|e| status::derive(e) == Status::Validated)
            .count(),
        20
    );
    // One undo takes the whole job back.
    undo.undo(&mut entries).unwrap();
    for (a, b) in entries.iter().zip(&before) {
        assert_eq!(a.trans, b.trans);
        assert_eq!(status::derive(a), status::derive(b));
        assert_eq!(a.prov, b.prov);
    }
}
