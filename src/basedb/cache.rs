//! One base index per (game, target language) pair, kept in memory with a
//! small LRU bound (E-12): documents of different games or targets can be
//! open at once without rescanning the base folders at every tab change.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use super::BaseDb;

struct Entry {
    game: String,
    target: String,
    db: Arc<BaseDb>,
    used: Instant,
}

pub struct BaseDbCache {
    entries: Vec<Entry>,
    cap: usize,
}

impl Default for BaseDbCache {
    fn default() -> Self {
        Self::new(3)
    }
}

impl BaseDbCache {
    pub fn new(cap: usize) -> Self {
        BaseDbCache {
            entries: Vec::new(),
            cap: cap.max(1),
        }
    }

    /// The index of `(game, target)`, built from `dirs` when not cached yet.
    /// `builder` is only called on a miss.
    pub fn get_or_build_with(
        &mut self,
        game: &str,
        target: &str,
        builder: impl FnOnce() -> BaseDb,
    ) -> Arc<BaseDb> {
        let game = game.trim();
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.game == game && e.target == target)
        {
            e.used = Instant::now();
            return e.db.clone();
        }
        let db = Arc::new(builder());
        if self.entries.len() >= self.cap {
            // Evict the least recently used.
            if let Some((i, _)) = self.entries.iter().enumerate().min_by_key(|(_, e)| e.used) {
                self.entries.remove(i);
            }
        }
        self.entries.push(Entry {
            game: game.to_string(),
            target: target.to_string(),
            db: db.clone(),
            used: Instant::now(),
        });
        db
    }

    /// The index of `(game, target)`, scanning `dirs` on a miss.
    pub fn get_or_build(&mut self, game: &str, target: &str, dirs: &[PathBuf]) -> Arc<BaseDb> {
        self.get_or_build_with(game, target, || BaseDb::scan(dirs, target))
    }

    /// Drop the indexes of one game (or of every game with `None`), e.g. after
    /// the base folders changed or a dictionary was written.
    pub fn invalidate(&mut self, game: Option<&str>) {
        match game {
            Some(g) => self.entries.retain(|e| e.game != g.trim()),
            None => self.entries.clear(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuse_eviction_and_invalidation() {
        let mut cache = BaseDbCache::new(2);
        let mut builds = 0;
        let a = cache.get_or_build_with("SkyrimSE", "fra", || {
            builds += 1;
            BaseDb::default()
        });
        let a2 = cache.get_or_build_with("SkyrimSE", "fra", || {
            builds += 1;
            BaseDb::default()
        });
        assert!(Arc::ptr_eq(&a, &a2));
        assert_eq!(builds, 1);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _b = cache.get_or_build_with("Fallout4", "fra", || {
            builds += 1;
            BaseDb::default()
        });
        assert_eq!(cache.len(), 2);
        // Touch A so B is the least recently used, then a third pair evicts B.
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _ = cache.get_or_build_with("SkyrimSE", "fra", || {
            builds += 1;
            BaseDb::default()
        });
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _c = cache.get_or_build_with("SkyrimSE", "deu", || {
            builds += 1;
            BaseDb::default()
        });
        assert_eq!(cache.len(), 2);
        assert_eq!(builds, 3);
        let _ = cache.get_or_build_with("Fallout4", "fra", || {
            builds += 1;
            BaseDb::default()
        });
        assert_eq!(builds, 4, "B was evicted and rebuilt");
        cache.invalidate(Some("SkyrimSE"));
        assert_eq!(cache.len(), 1);
        cache.invalidate(None);
        assert!(cache.is_empty());
    }
}

/// One index per dictionary file (B-02): the tree lists files in an order
/// the user chooses, so each file is indexed on its own and reloaded when
/// it changes on disk.
#[derive(Default)]
pub struct FileDbCache {
    entries: std::collections::HashMap<PathBuf, (Option<std::time::SystemTime>, Arc<BaseDb>)>,
}

impl FileDbCache {
    /// The index of `path`, read on a miss or when the file changed.
    pub fn get(&mut self, path: &std::path::Path) -> Option<Arc<BaseDb>> {
        let mtime = super::mtime(path);
        if let Some((t, db)) = self.entries.get(path)
            && *t == mtime
        {
            return Some(db.clone());
        }
        let db = Arc::new(BaseDb::from_file(path)?);
        self.entries.insert(path.to_path_buf(), (mtime, db.clone()));
        Some(db)
    }

    /// Whether `path` is indexed (without reading it).
    pub fn peek(&self, path: &std::path::Path) -> Option<Arc<BaseDb>> {
        self.entries.get(path).map(|(_, db)| db.clone())
    }

    pub fn invalidate(&mut self, path: Option<&std::path::Path>) {
        match path {
            Some(p) => {
                self.entries.remove(p);
            }
            None => self.entries.clear(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
