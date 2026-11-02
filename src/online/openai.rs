//! OpenAI and compatible endpoints (A.4, D-03): `POST /chat/completions`
//! with the prompt of the profile as the system message, the texts as a
//! JSON array (or separated lines) as the user message; `GET /models` at
//! the test. A local server (Ollama, LM Studio, vLLM…) works without a key.

use super::profile::{Answer, Profile};
use super::{Connector, Error, OnlineConfig, Service};

/// The connector of one profile.
pub struct OpenAI {
    pub profile: Profile,
}

impl OpenAI {
    pub fn new(profile: Profile) -> OpenAI {
        OpenAI { profile }
    }
}

/// The separator of the test's answer (a model name may hold `:`).
pub const TEST_SEP: char = '\u{1f}';

/// The JSON array of strings of a model's answer, code fences and
/// surrounding prose tolerated.
pub fn parse_answer(content: &str, n: usize) -> Result<Vec<String>, Error> {
    let t = content.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let start = t
        .find('[')
        .ok_or_else(|| Error::Format(crate::l10n!("err-online-format")))?;
    let end = t
        .rfind(']')
        .ok_or_else(|| Error::Format(crate::l10n!("err-online-format")))?;
    if end < start {
        return Err(Error::Format(crate::l10n!("err-online-format")));
    }
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(&t[start..=end]).map_err(|e| Error::Format(e.to_string()))?;
    let out: Vec<String> = arr
        .iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    if out.len() != n {
        return Err(Error::Incomplete {
            got: out.len(),
            sent: n,
        });
    }
    Ok(out)
}

/// The numbered marker of a text in « lines » form.
fn marker(i: usize) -> String {
    format!("⟨⟨{i}⟩⟩")
}

/// The texts in « lines » form (R-D03-03): each after its marker
/// « ⟨⟨n⟩⟩ », or separated by the profile's separator.
pub fn join_lines(texts: &[String], separator: &str) -> String {
    if separator.is_empty() {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{}\n{t}", marker(i + 1)))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        texts.join(&format!("\n{separator}\n"))
    }
}

/// The texts of an answer in « lines » form.
pub fn parse_lines(content: &str, separator: &str, n: usize) -> Result<Vec<String>, Error> {
    let t = content.trim();
    let out: Vec<String> = if separator.is_empty() {
        let re = regex::Regex::new(r"⟨⟨\s*(\d+)\s*⟩⟩").expect("valid regex");
        let marks: Vec<(usize, usize, usize)> = re
            .captures_iter(t)
            .filter_map(|c| {
                let m = c.get(0)?;
                Some((m.start(), m.end(), c[1].parse().ok()?))
            })
            .collect();
        let mut slots: Vec<Option<String>> = vec![None; n];
        for (k, (_, end, num)) in marks.iter().enumerate() {
            let stop = marks.get(k + 1).map(|m| m.0).unwrap_or(t.len());
            if *num >= 1 && *num <= n {
                slots[num - 1] = Some(t[*end..stop].trim().to_string());
            }
        }
        if slots.iter().any(Option::is_none) {
            return Err(Error::Incomplete {
                got: slots.iter().filter(|s| s.is_some()).count(),
                sent: n,
            });
        }
        slots.into_iter().flatten().collect()
    } else {
        t.split(separator).map(|s| s.trim().to_string()).collect()
    };
    if out.len() != n {
        return Err(Error::Incomplete {
            got: out.len(),
            sent: n,
        });
    }
    Ok(out)
}

/// An `f32` as the decimal written in the file (0.2, not 0.200000003).
fn round(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

impl OpenAI {
    /// The request's body (R-D03-02 / 04).
    pub fn body(
        &self,
        cfg: &OnlineConfig,
        texts: &[String],
        src: &str,
        dst: &str,
    ) -> serde_json::Value {
        let p = &self.profile;
        let user = match p.answer {
            Answer::Json => serde_json::to_string(texts).unwrap_or_default(),
            Answer::Lines => join_lines(texts, &p.separator),
        };
        let mut body = serde_json::json!({
            "model": p.model.trim(),
            "messages": [
                { "role": "system", "content": p.render_prompt(src, dst, &cfg.game, texts.len()) },
                { "role": "user", "content": user }
            ]
        });
        if let Some(o) = body.as_object_mut() {
            let num = |v: Option<f32>| v.map(|v| serde_json::json!(round(v)));
            for (k, v) in [
                ("temperature", num(p.temperature)),
                ("top_p", num(p.top_p)),
                ("max_tokens", p.max_tokens.map(|v| serde_json::json!(v))),
                ("frequency_penalty", num(p.frequency_penalty)),
                ("presence_penalty", num(p.presence_penalty)),
            ] {
                if let Some(v) = v {
                    o.insert(k.to_string(), v);
                }
            }
        }
        body
    }

    fn request(&self, cfg: &OnlineConfig, url: &str, key: &str, post: bool) -> ureq::Request {
        let agent = cfg.agent_for(url);
        let mut req = if post {
            agent.post(url)
        } else {
            agent.get(url)
        };
        if post {
            req = req.set("Content-Type", "application/json");
        }
        for (k, v) in &self.profile.headers {
            req = req.set(k, v);
        }
        if !key.trim().is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", key.trim()));
        }
        req
    }
}

impl Connector for OpenAI {
    fn service(&self) -> Service {
        Service::OpenAI(self.profile.id.clone())
    }

    fn translate(
        &self,
        cfg: &OnlineConfig,
        key: &str,
        texts: &[String],
        src: &str,
        dst: &str,
    ) -> Result<Vec<String>, Error> {
        let url = format!("{}/chat/completions", self.profile.base());
        let resp = self
            .request(cfg, &url, key, true)
            .send_json(self.body(cfg, texts, src, dst))
            .map_err(Error::from_ureq)?;
        let v: serde_json::Value = resp.into_json().map_err(|e| Error::Format(e.to_string()))?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| Error::Format(crate::l10n!("err-online-format")))?;
        match self.profile.answer {
            Answer::Json => parse_answer(content, texts.len()),
            Answer::Lines => parse_lines(content, &self.profile.separator, texts.len()),
        }
    }

    /// `GET /models` (the model absent of the list is said, not refused),
    /// then the sentence of control with the profile's prompt and form.
    fn test(&self, cfg: &OnlineConfig, key: &str) -> Result<String, Error> {
        let url = format!("{}/models", self.profile.base());
        let resp = self
            .request(cfg, &url, key, false)
            .call()
            .map_err(Error::from_ureq)?;
        let v: serde_json::Value = resp.into_json().map_err(|e| Error::Format(e.to_string()))?;
        let wanted = self.profile.model.trim();
        let known = v["data"]
            .as_array()
            .map(|a| a.iter().any(|m| m["id"].as_str() == Some(wanted)))
            .unwrap_or(false);
        let out = self.translate(cfg, key, &[super::CONTROL_SENTENCE.to_string()], "en", "fr")?;
        Ok(format!(
            "model:{wanted}{TEST_SEP}{}{TEST_SEP}{}",
            if known { "known" } else { "unlisted" },
            out.first().cloned().unwrap_or_default()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_answers() {
        assert_eq!(parse_answer(r#"["a","b"]"#, 2).unwrap(), vec!["a", "b"]);
        assert_eq!(
            parse_answer("```json\n[\"Bonjour <a_A>\"]\n```", 1).unwrap(),
            vec!["Bonjour <a_A>"]
        );
        assert_eq!(
            parse_answer("Here you go: [\"x\"] thanks", 1).unwrap(),
            vec!["x"]
        );
        assert!(parse_answer("nothing", 1).is_err());
        assert_eq!(
            parse_answer(r#"["a"]"#, 2),
            Err(Error::Incomplete { got: 1, sent: 2 })
        );
    }

    #[test]
    fn lines_with_markers_and_separators() {
        let texts = vec!["One".to_string(), "Two\nlines".to_string()];
        let j = join_lines(&texts, "");
        assert_eq!(j, "⟨⟨1⟩⟩\nOne\n⟨⟨2⟩⟩\nTwo\nlines");
        // A model that reorders or pads.
        let a = "Voici :\n⟨⟨2⟩⟩\nDeux\nlignes\n⟨⟨1⟩⟩ Un";
        assert_eq!(parse_lines(a, "", 2).unwrap(), vec!["Un", "Deux\nlignes"]);
        assert_eq!(
            parse_lines("⟨⟨1⟩⟩ Un", "", 2),
            Err(Error::Incomplete { got: 1, sent: 2 })
        );
        assert_eq!(join_lines(&texts, "|||"), "One\n|||\nTwo\nlines");
        assert_eq!(
            parse_lines("Un\n|||\nDeux", "|||", 2).unwrap(),
            vec!["Un", "Deux"]
        );
    }
}
