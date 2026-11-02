//! DeepL (A.4): `POST /v2/translate`, XML tag handling on the protection
//! tokens, formality; `GET /v2/usage` at the test.

use super::{Connector, Error, OnlineConfig, Service};

pub struct DeepL;

fn base(cfg: &OnlineConfig) -> String {
    let e = cfg.deepl_endpoint.trim().trim_end_matches('/');
    if e.starts_with("http") {
        e.to_string()
    } else {
        format!("https://{e}")
    }
}

impl Connector for DeepL {
    fn service(&self) -> Service {
        Service::DeepL
    }

    fn translate(
        &self,
        cfg: &OnlineConfig,
        key: &str,
        texts: &[String],
        src: &str,
        dst: &str,
    ) -> Result<Vec<String>, Error> {
        if key.trim().is_empty() {
            return Err(Error::NoKey);
        }
        let ignore = super::protect::token_names(texts);
        let mut body = serde_json::json!({
            "text": texts,
            "source_lang": src.to_uppercase(),
            "target_lang": dst.to_uppercase(),
            "tag_handling": "xml",
            "ignore_tags": ignore,
            "preserve_formatting": true,
        });
        let formality = cfg.deepl_formality.trim();
        if !formality.is_empty() && formality != "default" {
            body["formality"] = serde_json::Value::String(formality.to_string());
        }
        let url = format!("{}/v2/translate", base(cfg));
        let resp = cfg
            .agent_for(&url)
            .post(&url)
            .set("Authorization", &format!("DeepL-Auth-Key {}", key.trim()))
            .send_json(body)
            .map_err(Error::from_ureq)?;
        let v: serde_json::Value = resp.into_json().map_err(|e| Error::Format(e.to_string()))?;
        parse_translations(&v, texts.len())
    }

    fn test(&self, cfg: &OnlineConfig, key: &str) -> Result<String, Error> {
        if key.trim().is_empty() {
            return Err(Error::NoKey);
        }
        let url = format!("{}/v2/usage", base(cfg));
        let resp = cfg
            .agent_for(&url)
            .get(&url)
            .set("Authorization", &format!("DeepL-Auth-Key {}", key.trim()))
            .call()
            .map_err(Error::from_ureq)?;
        let v: serde_json::Value = resp.into_json().map_err(|e| Error::Format(e.to_string()))?;
        let used = v["character_count"].as_u64().unwrap_or(0);
        let limit = v["character_limit"].as_u64().unwrap_or(0);
        Ok(format!("usage:{used}/{limit}"))
    }
}

/// `{ "translations": [ { "text": … } ] }`.
pub fn parse_translations(v: &serde_json::Value, n: usize) -> Result<Vec<String>, Error> {
    let arr = v["translations"]
        .as_array()
        .ok_or_else(|| Error::Format(crate::l10n!("err-online-format")))?;
    let out: Vec<String> = arr
        .iter()
        .map(|t| t["text"].as_str().unwrap_or("").to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_deepl_answers() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"translations":[{"detected_source_language":"EN","text":"Bonjour <a_A>"},{"text":"Au revoir"}]}"#).unwrap();
        assert_eq!(
            parse_translations(&v, 2).unwrap(),
            vec!["Bonjour <a_A>", "Au revoir"]
        );
        assert!(parse_translations(&v, 3).is_err());
        let mut cfg = OnlineConfig::default();
        assert_eq!(base(&cfg), "https://api-free.deepl.com");
        cfg.deepl_endpoint = "https://api.deepl.com/".into();
        assert_eq!(base(&cfg), "https://api.deepl.com");
    }
}
