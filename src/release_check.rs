//! Checking for a newer version (E-14), as XIMOD Architect does: once a
//! day at start (`[Update] CheckUpdates`), or on request (Aide › Rechercher
//! les mises à jour…), one HTTPS request to the public GitHub API reads the
//! number of the latest published release. Nothing is sent about the user,
//! nothing is ever downloaded or installed: the interface shows a banner
//! with the links, the user chooses.

use std::time::Duration;

/// The repository of the application (`repository` of Cargo.toml).
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// A version number `major.minor.patch`; a pre-release suffix
/// (`0.5.0-beta.1`) is ignored, GitHub's « latest » never designates one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// `v0.4.1`, `0.4.1`, `0.4`, `V1.0.0-rc1`; `None` for anything else.
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s
            .strip_prefix('v')
            .or_else(|| s.strip_prefix('V'))
            .unwrap_or(s);
        let core = s.split(['-', '+', ' ']).next().unwrap_or("");
        let mut parts = core.split('.');
        let mut num = || -> Option<Option<u64>> {
            match parts.next() {
                None => Some(None),
                Some(p) => p.parse::<u64>().ok().map(Some),
            }
        };
        let major = num()??;
        let minor = num()?.unwrap_or(0);
        let patch = num()?.unwrap_or(0);
        if parts.next().is_some() {
            return None;
        }
        Some(Version(major, minor, patch))
    }

    /// The version of this build.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Version(0, 0, 0))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The page of the release on GitHub.
    pub url: String,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A version newer than this one.
    Newer(Release),
    /// This version is the latest (or newer: a development build).
    UpToDate(Version),
    /// The request or the answer failed (the text, in English, for the log).
    Failed(String),
}

/// `owner/repo` of a GitHub URL.
fn owner_repo(repository: &str) -> Option<String> {
    let rest = repository
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split("github.com/")
        .nth(1)?;
    let mut it = rest.split('/');
    let (o, r) = (it.next()?, it.next()?);
    (!o.is_empty() && !r.is_empty()).then(|| format!("{o}/{r}"))
}

/// The API address of the latest release. `XIMOD_RELEASES_API` replaces it
/// (a local server, to try the banner without publishing).
pub fn api_url() -> String {
    if let Ok(u) = std::env::var("XIMOD_RELEASES_API")
        && u.starts_with("http")
    {
        return u;
    }
    let repo = owner_repo(REPOSITORY).unwrap_or_else(|| "plplecuyer/ximod-translator".into());
    format!("https://api.github.com/repos/{repo}/releases/latest")
}

/// The page listing the releases, for the links when the answer gives none.
pub fn releases_page() -> String {
    format!("{}/releases/latest", REPOSITORY.trim_end_matches('/'))
}

/// The release described by the JSON of `releases/latest`.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if v.get("draft").and_then(|d| d.as_bool()) == Some(true)
        || v.get("prerelease").and_then(|d| d.as_bool()) == Some(true)
    {
        return Err("not a published release".into());
    }
    let tag = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or("no tag_name")?;
    let version = Version::parse(tag).ok_or_else(|| format!("tag {tag:?} is not a version"))?;
    let url = v
        .get("html_url")
        .and_then(|u| u.as_str())
        .filter(|u| u.starts_with("https://github.com/"))
        .map(str::to_string)
        .unwrap_or_else(releases_page);
    Ok(Release { version, url })
}

/// The outcome of a release for the version `current`.
pub fn compare(current: Version, latest: Release) -> Outcome {
    if latest.version > current {
        Outcome::Newer(latest)
    } else {
        Outcome::UpToDate(latest.version)
    }
}

/// Whether the daily check is due: never made, or made another day
/// (`YYYY-MM-DD`).
pub fn is_due(last_check: &str, today: &str) -> bool {
    last_check.trim() != today.trim()
}

/// Ask GitHub (blocking: call it off the interface thread). `proxy`: the
/// proxy of the online settings, if any.
pub fn check(proxy: Option<&str>, timeout: Duration) -> Outcome {
    let mut b = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("XIMOD-Translator/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false);
    let url = api_url();
    if let Some(p) = proxy
        && !crate::online::bypass_proxy(&url)
        && let Ok(proxy) = ureq::Proxy::new(p)
    {
        b = b.proxy(Some(proxy));
    }
    let agent: ureq::Agent = b.build().into();
    let answer = agent
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call();
    let body = match answer {
        // 404: no release published yet.
        Ok(r) if r.status().as_u16() == 404 => return Outcome::UpToDate(Version::current()),
        Ok(r) if r.status().as_u16() >= 400 => {
            return Outcome::Failed(format!("HTTP {}", r.status().as_u16()));
        }
        Ok(mut r) => match r.body_mut().read_to_string() {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(e.to_string()),
        },
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    match parse_release(&body) {
        Ok(r) => compare(Version::current(), r),
        Err(e) => Outcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_order() {
        assert_eq!(Version::parse("v0.4.1"), Some(Version(0, 4, 1)));
        assert_eq!(Version::parse("0.4"), Some(Version(0, 4, 0)));
        assert_eq!(Version::parse("V1.0.0-rc1"), Some(Version(1, 0, 0)));
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::parse(""), None);
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_api_follows_the_repository() {
        assert_eq!(
            owner_repo("https://github.com/plplecuyer/ximod-translator"),
            Some("plplecuyer/ximod-translator".into())
        );
        assert_eq!(
            owner_repo("https://github.com/a/b.git/"),
            Some("a/b".into())
        );
        assert_eq!(owner_repo("https://example.org/x"), None);
        assert!(api_url().starts_with("https://api.github.com/repos/"));
        assert!(api_url().ends_with("/releases/latest"));
        assert!(releases_page().starts_with("https://github.com/"));
    }

    #[test]
    fn a_release_answer_is_read() {
        let json = r#"{"tag_name":"v0.5.0","name":"XIMOD Translator 0.5.0",
            "html_url":"https://github.com/plplecuyer/ximod-translator/releases/tag/v0.5.0",
            "draft":false,"prerelease":false,"assets":[]}"#;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, Version(0, 5, 0));
        assert!(r.url.ends_with("/v0.5.0"));
        assert_eq!(
            compare(Version(0, 4, 0), r.clone()),
            Outcome::Newer(r.clone())
        );
        assert_eq!(
            compare(Version(0, 5, 0), r.clone()),
            Outcome::UpToDate(Version(0, 5, 0))
        );
        // A development build ahead of the release is up to date.
        assert_eq!(
            compare(Version(0, 6, 0), r),
            Outcome::UpToDate(Version(0, 5, 0))
        );
        // A link elsewhere than GitHub is not followed.
        let odd = r#"{"tag_name":"0.5.0","html_url":"https://evil.example/x"}"#;
        assert_eq!(parse_release(odd).unwrap().url, releases_page());
        assert!(parse_release(r#"{"tag_name":"v1.0","prerelease":true}"#).is_err());
        assert!(parse_release(r#"{"message":"Not Found"}"#).is_err());
        assert!(parse_release("not json").is_err());
    }

    #[test]
    fn the_messages_exist() {
        // The interface's keys, in the reference language (the others are
        // checked against it by the messages test).
        let eng = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/locales/eng/main.ftl"),
        )
        .unwrap();
        for k in [
            "menu-check-updates",
            "settings-check-updates",
            "settings-check-updates-hint",
            "release-banner",
            "release-link-github",
            "release-link-nexus",
            "release-dismiss",
            "release-title",
            "release-checking",
            "release-uptodate",
            "release-failed",
            "release-journal-newer",
        ] {
            assert!(eng.contains(&format!("\n{k} = ")), "{k}");
        }
    }

    #[test]
    fn the_check_is_due_once_a_day() {
        assert!(is_due("", "2026-11-13"));
        assert!(is_due("2026-11-12", "2026-11-13"));
        assert!(!is_due("2026-11-13", "2026-11-13"));
    }
}
