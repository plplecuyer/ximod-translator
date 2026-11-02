//! The API keys (R-D02-05, decision D-28): the system keyring through
//! `keyring` (Credential Manager, Keychain, Secret Service), else
//! `secrets.json` of the configuration folder with the user's rights only;
//! never `Config.ini`, never a log.

use std::collections::HashMap;
use std::path::PathBuf;

use super::Service;

const KEYRING_SERVICE: &str = "XIMOD Translator";

/// Where a key is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    Keyring,
    File,
    None,
}

fn file_path() -> PathBuf {
    crate::config::paths::get().config_dir.join("secrets.json")
}

fn read_file() -> HashMap<String, String> {
    std::fs::read_to_string(file_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_file(map: &HashMap<String, String>) -> Result<(), String> {
    let p = file_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(map).map_err(|e| e.to_string())?;
    crate::config::paths::write_atomic(&p, &json).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn entry_of(id: &str) -> Option<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, id).ok()
}

/// Whether the keyring answers at all (a probe, cached for the session).
pub fn keyring_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, "probe") else {
            return false;
        };
        match e.get_password() {
            Ok(_) => true,
            Err(keyring::Error::NoEntry) => true,
            Err(_) => false,
        }
    })
}

/// The key of a service and where it was found; the « openai » profile
/// takes the key of the first version (`openai`) until it has its own
/// (R-D03-05).
pub fn get(service: &Service) -> (Option<String>, Store) {
    let r = get_id(&service.id());
    if r.0.is_none() && *service == Service::OpenAI("openai".into()) {
        return get_id(LEGACY_OPENAI);
    }
    r
}

/// The id of the OpenAI key of the first version.
const LEGACY_OPENAI: &str = "openai";

fn get_id(id: &str) -> (Option<String>, Store) {
    if keyring_available()
        && let Some(e) = entry_of(id)
        && let Ok(k) = e.get_password()
        && !k.is_empty()
    {
        return (Some(k), Store::Keyring);
    }
    match read_file().remove(id) {
        Some(k) if !k.is_empty() => (Some(k), Store::File),
        _ => (None, Store::None),
    }
}

/// Store a key (empty = remove); says where it went.
pub fn set(service: &Service, key: &str) -> Result<Store, String> {
    if *service == Service::OpenAI("openai".into()) {
        // The key of the first version gives way to the profile's.
        let _ = set_id(LEGACY_OPENAI, "");
    }
    set_id(&service.id(), key)
}

fn set_id(id: &str, key: &str) -> Result<Store, String> {
    let key = key.trim();
    if keyring_available()
        && let Some(e) = entry_of(id)
    {
        let r = if key.is_empty() {
            match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(err),
            }
        } else {
            e.set_password(key)
        };
        if r.is_ok() {
            // A stale copy in the file must not shadow the keyring.
            let mut m = read_file();
            if m.remove(id).is_some() {
                let _ = write_file(&m);
            }
            return Ok(if key.is_empty() {
                Store::None
            } else {
                Store::Keyring
            });
        }
    }
    let mut m = read_file();
    if key.is_empty() {
        m.remove(id);
    } else {
        m.insert(id.to_string(), key.to_string());
    }
    write_file(&m)?;
    Ok(if key.is_empty() {
        Store::None
    } else {
        Store::File
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trip() {
        // The configuration folder of the tests is a scratch one; the file
        // store is exercised whatever the keyring says by writing directly.
        let mut m = HashMap::new();
        m.insert("deepl".to_string(), "abc:fx".to_string());
        write_file(&m).unwrap();
        assert_eq!(read_file().get("deepl").map(String::as_str), Some("abc:fx"));
        m.clear();
        write_file(&m).unwrap();
        assert!(read_file().is_empty());
    }
}
