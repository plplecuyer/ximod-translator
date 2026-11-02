//! Azure Translator (A.4): `POST /translate?api-version=3.0`, html text
//! type on the protection tokens, region header.

use super::{Connector, Error, OnlineConfig, Service};

pub struct Azure;

fn base(cfg: &OnlineConfig) -> String {
    let e = cfg.azure_endpoint.trim().trim_end_matches('/');
    if e.is_empty() {
        "https://api.cognitive.microsofttranslator.com".to_string()
    } else if e.starts_with("http") {
        e.to_string()
    } else {
        format!("https://{e}")
    }
}

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
    let body: Vec<serde_json::Value> = texts
        .iter()
        .map(|t| serde_json::json!({ "Text": t }))
        .collect();
    let url = format!(
        "{}/translate?api-version=3.0&from={src}&to={dst}&textType=html",
        base(cfg)
    );
    let mut req = cfg
        .agent_for(&url)
        .post(&url)
        .set("Ocp-Apim-Subscription-Key", key.trim())
        .set("Content-Type", "application/json");
    if !cfg.azure_region.trim().is_empty() {
        req = req.set("Ocp-Apim-Subscription-Region", cfg.azure_region.trim());
    }
    let resp = req
        .send_json(serde_json::Value::Array(body))
        .map_err(Error::from_ureq)?;
    let v: serde_json::Value = resp.into_json().map_err(|e| Error::Format(e.to_string()))?;
    parse_answers(&v, texts.len())
}

/// `[ { "translations": [ { "text": …, "to": "fr" } ] } ]`.
pub fn parse_answers(v: &serde_json::Value, n: usize) -> Result<Vec<String>, Error> {
    let arr = v
        .as_array()
        .ok_or_else(|| Error::Format(crate::l10n!("err-online-format")))?;
    let out: Vec<String> = arr
        .iter()
        .map(|item| {
            item["translations"][0]["text"]
                .as_str()
                .map(super::google::unescape_html)
                .unwrap_or_default()
        })
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

impl Connector for Azure {
    fn service(&self) -> Service {
        Service::Azure
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
    fn parses_azure_answers() {
        let v: serde_json::Value =
            serde_json::from_str(r#"[{"translations":[{"text":"Le dragon arrive.","to":"fr"}]}]"#)
                .unwrap();
        assert_eq!(parse_answers(&v, 1).unwrap(), vec!["Le dragon arrive."]);
        assert!(parse_answers(&v, 2).is_err());
    }
}
