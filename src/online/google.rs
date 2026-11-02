//! Google Cloud Translation v2 (A.4): `POST /language/translate/v2` with an
//! API key, html format on the protection tokens.

use super::{Connector, Error, OnlineConfig, Service};

pub struct Google;

const URL: &str = "https://translation.googleapis.com/language/translate/v2";

fn call(
    cfg: &OnlineConfig,
    key: &str,
    texts: &[String],
    src: &str,
    dst: &str,
) -> Result<Vec<String>, Error> {
    if key.trim().is_empty() {
        return Err(Error::NoKey);
    }
    let body = serde_json::json!({
        "q": texts,
        "source": src,
        "target": dst,
        "format": "html",
    });
    let url = format!("{URL}?key={}", key.trim());
    let resp = cfg
        .agent_for(&url)
        .post(&url)
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(Error::from_ureq)?;
    let v: serde_json::Value = resp.into_json().map_err(|e| Error::Format(e.to_string()))?;
    parse_answers(&v, texts.len())
}

/// `{ "data": { "translations": [ { "translatedText": … } ] } }`.
pub fn parse_answers(v: &serde_json::Value, n: usize) -> Result<Vec<String>, Error> {
    let arr = v["data"]["translations"]
        .as_array()
        .ok_or_else(|| Error::Format(crate::l10n!("err-online-format")))?;
    let out: Vec<String> = arr
        .iter()
        .map(|t| unescape_html(t["translatedText"].as_str().unwrap_or("")))
        .collect();
    if out.len() != n {
        return Err(Error::Format(crate::l10n!(
            "err-online-count",
            got = out.len(),
            n = n
        )));
    }
    Ok(out)
}

/// The entities the html answers carry (`&#39;`, `&quot;`, `&amp;`…).
pub fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let ent = &tail[1..end];
        let rep = match ent {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" => Some("'".to_string()),
            "nbsp" => Some("\u{A0}".to_string()),
            _ => ent
                .strip_prefix('#')
                .and_then(|num| {
                    if let Some(h) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32)
                .map(|c| c.to_string()),
        };
        match rep {
            Some(r) if end <= 10 => {
                out.push_str(&r);
                rest = &tail[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

impl Connector for Google {
    fn service(&self) -> Service {
        Service::Google
    }

    fn translate(
        &self,
        cfg: &OnlineConfig,
        key: &str,
        texts: &[String],
        src: &str,
        dst: &str,
    ) -> Result<Vec<String>, Error> {
        call(cfg, key, texts, src, dst)
    }

    fn test(&self, cfg: &OnlineConfig, key: &str) -> Result<String, Error> {
        let out = call(cfg, key, &[super::CONTROL_SENTENCE.to_string()], "en", "fr")?;
        Ok(format!(
            "control:{}",
            out.first().cloned().unwrap_or_default()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_unescapes() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"data":{"translations":[{"translatedText":"L&#39;épée de <a_A> &amp; co","detectedSourceLanguage":"en"}]}}"#,
        )
        .unwrap();
        assert_eq!(parse_answers(&v, 1).unwrap(), vec!["L'épée de <a_A> & co"]);
        assert_eq!(
            unescape_html("a &lt;b&gt; &quot;c&quot; &#x41; & d"),
            "a <b> \"c\" A & d"
        );
        assert_eq!(unescape_html("no entity"), "no entity");
    }
}
