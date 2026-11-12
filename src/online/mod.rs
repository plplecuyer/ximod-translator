//! Online translation (D-01, D-02): the services — DeepL, Azure Translator,
//! OpenAI and compatible endpoints, Google Cloud Translation — behind one
//! trait, a blocking HTTP client (`ureq`, rustls) run in a background
//! thread, the protection of what must not be translated, the quotas, the
//! cache of the answers and the keys in the system keyring.
//!
//! Nothing is sent without an explicit action of the user; every answer
//! comes back « En attente » with its provenance (R-D01-04).

pub mod azure;
pub mod browser;
pub mod cache;
pub mod deepl;
pub mod google;
pub mod job;
pub mod mass;
pub mod openai;
pub mod profile;
pub mod protect;
pub mod quota;
pub mod secrets;

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The services (A.4): DeepL, Azure, Google, and one OpenAI-compatible
/// service per profile (D-03, D-85).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Service {
    #[default]
    DeepL,
    Azure,
    /// The profile's id.
    OpenAI(String),
    Google,
}

impl Service {
    /// The services of the session: DeepL, Azure, the profiles, Google.
    pub fn all() -> Vec<Service> {
        let mut v = vec![Service::DeepL, Service::Azure];
        v.extend(profile::all().into_iter().map(|p| Service::OpenAI(p.id)));
        v.push(Service::Google);
        v
    }

    /// The id in `Config.ini`, the keyring and the provenance (`deepl`,
    /// `openai:local`…).
    pub fn id(&self) -> String {
        match self {
            Service::DeepL => "deepl".into(),
            Service::Azure => "azure".into(),
            Service::OpenAI(p) => format!("openai:{p}"),
            Service::Google => "google".into(),
        }
    }

    /// The name of the cache file (`openai-local`; the « openai » profile
    /// keeps the cache of the first version, `openai`).
    pub fn file_id(&self) -> String {
        match self {
            Service::OpenAI(p) if p == "openai" => "openai".into(),
            Service::OpenAI(p) => format!("openai-{p}"),
            other => other.id(),
        }
    }

    /// `openai` (the first version) is the « openai » profile.
    pub fn from_id(s: &str) -> Option<Service> {
        let s = s.trim();
        Some(match s {
            "deepl" => Service::DeepL,
            "azure" => Service::Azure,
            "google" => Service::Google,
            "openai" => Service::OpenAI("openai".into()),
            _ => {
                let p = s.strip_prefix("openai:")?;
                if !profile::is_id(p) {
                    return None;
                }
                Service::OpenAI(p.to_string())
            }
        })
    }

    /// The i18n key of the name of DeepL, Azure, Google.
    pub fn label_key(&self) -> &'static str {
        match self {
            Service::DeepL => "online-service-deepl",
            Service::Azure => "online-service-azure",
            Service::OpenAI(_) => "online-service-openai",
            Service::Google => "online-service-google",
        }
    }

    /// The name of a profile (its id when it is gone).
    pub fn profile_name(&self) -> Option<String> {
        match self {
            Service::OpenAI(id) => Some(
                profile::get(id)
                    .map(|p| p.name)
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| id.clone()),
            ),
            _ => None,
        }
    }

    /// The name shown in the lists: « OpenAI — Ollama local ».
    pub fn label(&self, i18n: &crate::i18n::I18n) -> String {
        match self.profile_name() {
            Some(n) => i18n.t_arg("online-service-profile", "name", &n),
            None => i18n.t(self.label_key()),
        }
    }

    /// The name in a provenance: the profile's, else the service's.
    pub fn short_label(&self, i18n: &crate::i18n::I18n) -> String {
        self.profile_name()
            .unwrap_or_else(|| i18n.t(self.label_key()))
    }

    /// Whether the service works without a key (a local server).
    pub fn key_optional(&self) -> bool {
        match self {
            Service::OpenAI(id) => profile::get(id).is_none_or(|p| p.key_optional),
            _ => false,
        }
    }

    /// The profile of an OpenAI-compatible service.
    pub fn profile(&self) -> Option<profile::Profile> {
        match self {
            Service::OpenAI(id) => profile::get(id),
            _ => None,
        }
    }
}

/// The settings of `[Online]` (R-D01-01, R-D02-04, R-D02-06).
#[derive(Debug, Clone, PartialEq)]
pub struct OnlineConfig {
    pub service: Option<Service>,
    pub deepl_endpoint: String,
    pub deepl_formality: String,
    pub azure_region: String,
    pub azure_endpoint: String,
    pub openai_endpoint: String,
    pub openai_model: String,
    pub max_chars: usize,
    pub max_lines: usize,
    pub max_rpm: u32,
    pub session_warn: usize,
    pub protect_tags: bool,
    pub protect_numbers: bool,
    pub protect_newlines: bool,
    pub proxy: String,
    pub timeout_secs: u64,
    /// The name of the document's game, for `%game%` (not saved).
    pub game: String,
    /// The scope of « Traduire en ligne » chosen last (R-D04-01).
    pub last_scope: String,
    /// The page Ctrl+G opens without a service (R-T03, D-117).
    pub browser_site: browser::Site,
}

impl Default for OnlineConfig {
    fn default() -> Self {
        OnlineConfig {
            service: None,
            deepl_endpoint: "api-free.deepl.com".into(),
            deepl_formality: "default".into(),
            azure_region: String::new(),
            azure_endpoint: "https://api.cognitive.microsofttranslator.com".into(),
            openai_endpoint: "https://api.openai.com/v1".into(),
            openai_model: "gpt-4o-mini".into(),
            max_chars: 5000,
            max_lines: 50,
            max_rpm: 20,
            session_warn: 200_000,
            protect_tags: true,
            protect_numbers: true,
            protect_newlines: true,
            proxy: String::new(),
            timeout_secs: 30,
            game: String::new(),
            last_scope: "document".into(),
            browser_site: browser::Site::Google,
        }
    }
}

impl OnlineConfig {
    /// The settings for a service: a profile's guards replace the common
    /// ones (R-D03-06).
    pub fn for_service(&self, s: &Service) -> OnlineConfig {
        let mut c = self.clone();
        if let Some(p) = s.profile() {
            if let Some(v) = p.max_lines {
                c.max_lines = v;
            }
            if let Some(v) = p.max_chars {
                c.max_chars = v;
            }
            if let Some(v) = p.max_rpm {
                c.max_rpm = v;
            }
            if let Some(v) = p.timeout {
                c.timeout_secs = v;
            }
        }
        c
    }

    /// The proxy to use: the setting, else `HTTPS_PROXY` / `https_proxy`.
    pub fn effective_proxy(&self) -> Option<String> {
        let p = self.proxy.trim();
        if !p.is_empty() {
            return Some(p.to_string());
        }
        std::env::var("HTTPS_PROXY")
            .or_else(|_| std::env::var("https_proxy"))
            .ok()
            .filter(|s| !s.trim().is_empty())
    }

    /// The HTTP agent of a request to `url` (R-D01-07): the proxy, except
    /// for a local server (loopback, or a host listed in `NO_PROXY`).
    pub fn agent_for(&self, url: &str) -> ureq::Agent {
        let mut b = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(self.timeout_secs.max(5)))
            .user_agent("XIMOD-Translator/1.0");
        if !bypass_proxy(url)
            && let Some(p) = self.effective_proxy()
            && let Ok(proxy) = ureq::Proxy::new(&p)
        {
            b = b.proxy(proxy);
        }
        b.build()
    }
}

/// The host of a URL.
pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let hostport = rest.split(['/', '?']).next().unwrap_or(rest);
    let hostport = hostport.rsplit('@').next().unwrap_or(hostport);
    let host = if let Some(h) = hostport.strip_prefix('[') {
        h.split(']').next().unwrap_or(h)
    } else {
        hostport.split(':').next().unwrap_or(hostport)
    };
    host.to_lowercase()
}

/// A local server, or a host of `NO_PROXY`, is reached directly.
pub fn bypass_proxy(url: &str) -> bool {
    let host = host_of(url);
    if host == "localhost" || host == "::1" || host.starts_with("127.") || host.ends_with(".local")
    {
        return true;
    }
    let no = std::env::var("NO_PROXY")
        .or_else(|_| std::env::var("no_proxy"))
        .unwrap_or_default();
    no.split(',')
        .map(|h| h.trim().trim_start_matches('.').to_lowercase())
        .filter(|h| !h.is_empty())
        .any(|h| h == "*" || host == h || host.ends_with(&format!(".{h}")))
}

/// What went wrong (shown with the service's code and message, never the
/// key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The service answered with an error status.
    Http { code: u16, message: String },
    /// No answer (network, timeout, proxy).
    Network(String),
    /// An answer the connector could not read.
    Format(String),
    /// No key for a service that needs one.
    NoKey,
    /// The language pair is not covered by the service.
    Pair(String),
    /// A line longer than a request allows (never sent).
    TooLong,
    /// A model answered fewer or more texts than sent (R-D03-03).
    Incomplete { got: usize, sent: usize },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Http { code, message } => write!(f, "{code} {message}"),
            Error::Network(m) => write!(f, "{m}"),
            Error::Format(m) => write!(f, "{m}"),
            Error::NoKey => write!(f, "{}", crate::l10n!("err-online-no-key")),
            Error::Pair(m) => write!(f, "{m}"),
            Error::TooLong => write!(f, "{}", crate::l10n!("online-line-too-long")),
            Error::Incomplete { got, sent } => write!(
                f,
                "{}",
                crate::l10n!("online-answer-incomplete", got = got, sent = sent)
            ),
        }
    }
}

impl Error {
    /// The code shown (0 for a local failure).
    pub fn code(&self) -> u16 {
        match self {
            Error::Http { code, .. } => *code,
            _ => 0,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Error::Http { message, .. } => message.clone(),
            other => other.to_string(),
        }
    }

    /// Map a `ureq` failure, keeping the service's message (its body, cut)
    /// and never the request.
    pub fn from_ureq(e: ureq::Error) -> Error {
        match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let message = body_message(&body);
                Error::Http { code, message }
            }
            ureq::Error::Transport(t) => Error::Network(t.to_string()),
        }
    }
}

/// A readable message from an error body: the `message` / `error.message`
/// / `error` field of a JSON body, else the body itself cut at 200.
pub fn body_message(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        for path in [
            &["message"][..],
            &["error", "message"],
            &["error"],
            &["detail"],
        ] {
            let mut cur = &v;
            let mut ok = true;
            for k in path {
                match cur.get(k) {
                    Some(n) => cur = n,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok && let Some(s) = cur.as_str() {
                return s.to_string();
            }
        }
    }
    let t: String = body.trim().chars().take(200).collect();
    if t.is_empty() {
        crate::l10n!("err-online-no-message")
    } else {
        t
    }
}

/// One service's contract (A.4).
pub trait Connector: Send + Sync {
    fn service(&self) -> Service;

    /// Translate texts (already protected) from `src` to `dst` (ISO
    /// 639-1 codes, lowercase); the answers come in the same order.
    fn translate(
        &self,
        cfg: &OnlineConfig,
        key: &str,
        texts: &[String],
        src: &str,
        dst: &str,
    ) -> Result<Vec<String>, Error>;

    /// « Tester… » (R-D01-01): a sentence of control and what the service
    /// says of the account, as one line.
    fn test(&self, cfg: &OnlineConfig, key: &str) -> Result<String, Error>;
}

/// The connector of a service.
pub fn connector(service: &Service) -> Box<dyn Connector> {
    match service {
        Service::DeepL => Box::new(deepl::DeepL),
        Service::Azure => Box::new(azure::Azure),
        Service::OpenAI(id) => Box::new(openai::OpenAI::new(
            profile::get(id)
                .or_else(|| profile::get("openai"))
                .unwrap_or_default(),
        )),
        Service::Google => Box::new(google::Google),
    }
}

/// The language code a service takes: ISO 639-1 from the application's
/// ISO 639-3, lowercase (DeepL wants uppercase, its connector adapts).
pub fn iso1(code: &str) -> String {
    let c = code.trim().to_lowercase();
    if c.len() == 2 {
        return c;
    }
    crate::i18n::iso639_3_to_1(&c).to_lowercase()
}

/// The sentence every « Tester… » sends.
pub const CONTROL_SENTENCE: &str = "The dragon is coming.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_messages_and_ids() {
        assert_eq!(
            body_message(r#"{"message":"Quota exceeded"}"#),
            "Quota exceeded"
        );
        assert_eq!(
            body_message(r#"{"error":{"message":"Invalid key","code":401}}"#),
            "Invalid key"
        );
        assert_eq!(body_message(r#"{"error":"bad"}"#), "bad");
        assert_eq!(body_message("plain text"), "plain text");
        assert_eq!(body_message(""), crate::l10n!("err-online-no-message"));
        assert_eq!(Service::from_id("deepl"), Some(Service::DeepL));
        assert_eq!(Service::from_id("x"), None);
        assert_eq!(iso1("fra"), "fr");
        assert_eq!(iso1("en"), "en");
        let cfg = OnlineConfig::default();
        assert_eq!(cfg.max_chars, 5000);
        assert_eq!(
            host_of("https://api-free.deepl.com/v2/translate"),
            "api-free.deepl.com"
        );
        assert_eq!(host_of("http://127.0.0.1:8765/v1/models"), "127.0.0.1");
        assert_eq!(host_of("http://user:pw@[::1]:80/x"), "::1");
        assert!(bypass_proxy("http://localhost:1234/v1"));
        assert!(bypass_proxy("http://127.0.0.1:1234/v1"));
        assert!(!bypass_proxy("https://api.deepl.com/v2/translate"));
        assert!(
            Error::Http {
                code: 456,
                message: "quota".into()
            }
            .to_string()
            .starts_with("456")
        );
    }
}
