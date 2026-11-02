//! The cache of the answers (R-D01-05, decision D-38):
//! `cache/online/<service>_<SRC>-<DST>.ximdb` of the configuration
//! folder, protected text → answer, so a source is not paid twice.

use std::collections::HashMap;
use std::path::PathBuf;

use super::Service;
use crate::basedb::BaseRow;

pub struct Cache {
    path: PathBuf,
    lang: String,
    map: HashMap<String, String>,
    dirty: bool,
}

/// `<config>/cache/online`.
pub fn dir() -> PathBuf {
    crate::config::paths::get()
        .config_dir
        .join("cache")
        .join("online")
}

impl Cache {
    pub fn path_for(service: &Service, src: &str, dst: &str) -> PathBuf {
        dir().join(format!(
            "{}_{}-{}.ximdb",
            service.file_id(),
            src.to_uppercase(),
            dst.to_uppercase()
        ))
    }

    /// Open (or start) the cache of a pair.
    pub fn open(service: &Service, src: &str, dst: &str) -> Cache {
        Self::open_at(Self::path_for(service, src, dst), dst)
    }

    pub fn open_at(path: PathBuf, dst: &str) -> Cache {
        let mut map = HashMap::new();
        if path.is_file()
            && let Ok((_, rows, _)) = crate::basedb::read_rows_and_old(&path)
        {
            for r in rows {
                map.insert(r.source, r.trans);
            }
        }
        Cache {
            path,
            lang: dst.to_string(),
            map,
            dirty: false,
        }
    }

    pub fn get(&self, sent: &str) -> Option<&str> {
        self.map.get(sent).map(String::as_str)
    }

    pub fn put(&mut self, sent: &str, answer: &str) {
        if self.map.get(sent).map(String::as_str) != Some(answer) {
            self.map.insert(sent.to_string(), answer.to_string());
            self.dirty = true;
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Write when something changed.
    pub fn save(&mut self) -> Result<(), String> {
        if !self.dirty {
            return Ok(());
        }
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let mut rows: Vec<BaseRow> = self
            .map
            .iter()
            .map(|(s, t)| BaseRow {
                rec: "ONLINE".into(),
                field: "TXT".into(),
                formid: 0,
                edid: String::new(),
                source: s.clone(),
                trans: t.clone(),
                status: None,
                ..Default::default()
            })
            .collect();
        rows.sort_by(|a, b| a.source.cmp(&b.source));
        crate::basedb::write_ximdb(rows, &self.lang, &self.path)?;
        self.dirty = false;
        Ok(())
    }

    /// Paramètres ▸ Vider le cache: every file of the folder.
    pub fn clear_all() -> usize {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(dir()) {
            for e in rd.flatten() {
                if e.path().extension().is_some_and(|x| x == "ximdb")
                    && std::fs::remove_file(e.path()).is_ok()
                {
                    n += 1;
                }
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trips_through_ximdb() {
        let d = std::env::temp_dir().join(format!("ximod_online_cache_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("deepl_ENG-FRA.ximdb");
        let mut c = Cache::open_at(p.clone(), "fra");
        assert!(c.is_empty());
        c.put("Hello <a_A>", "Bonjour <a_A>");
        c.put("Bye", "Au revoir");
        c.save().unwrap();
        let c2 = Cache::open_at(p, "fra");
        assert_eq!(c2.len(), 2);
        assert_eq!(c2.get("Hello <a_A>"), Some("Bonjour <a_A>"));
        assert_eq!(c2.get("nope"), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
