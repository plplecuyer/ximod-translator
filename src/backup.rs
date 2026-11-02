//! Safeguards of the user's work (B-11).
//!
//! Two distinct mechanisms. **Timestamped copies** ([`rotate_backup`]) protect
//! a mod dictionary from being overwritten by a wrong version: the previous
//! file goes to `backups/<nom>_<AAAAMMJJ-HHMMSS>.ximdb` and the N most recent
//! copies are kept (R-B11-01). **Autosave** ([`Autosave`]) protects the work
//! in progress against a crash: every M minutes, each modified document is
//! written as a pivot JSON in the rescue folder with a manifest, from a
//! background thread and atomically (R-B11-02); at the next start the
//! manifests found are offered for resumption (R-B11-05).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::paths::write_atomic;
use crate::doc_settings::DocSettings;
use crate::tm::{TmEntry, TranslationMemory};

/// Default values of `[Backup]`.
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 5;
pub const AUTOSAVE_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 1..=60;
pub const DEFAULT_COPIES: usize = 10;
pub const COPIES_RANGE: std::ops::RangeInclusive<usize> = 0..=100;
/// Rescue files older than this are purged at start (R-B11-05).
pub const PURGE_AFTER_DAYS: u64 = 30;
/// Bound of the rescue folder (R-B11-07).
pub const QUOTA_BYTES: u64 = 500 * 1024 * 1024;

const MANIFEST_FORMAT: u32 = 1;

// ---------------------------------------------------------------- time

/// Local calendar time `(year, month, day, hour, minute, second)` of a Unix
/// timestamp, through the platform's own conversion (no date crate).
pub fn local_datetime(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn localtime_r(t: *const i64, out: *mut u8) -> *mut u8;
        }
        // `struct tm` starts with nine C ints on every Unix (sec, min, hour,
        // mday, mon, year, wday, yday, isdst); the buffer is oversized for
        // the platform-specific tail (gmtoff, zone).
        let mut buf = [0u8; 256];
        let t: i64 = secs;
        // SAFETY: localtime_r writes at most sizeof(struct tm) < 256 bytes.
        let ok = unsafe { !localtime_r(&t, buf.as_mut_ptr()).is_null() };
        if ok {
            let int = |i: usize| {
                i32::from_ne_bytes([buf[i * 4], buf[i * 4 + 1], buf[i * 4 + 2], buf[i * 4 + 3]])
            };
            return (
                1900 + int(5),
                (int(4) + 1) as u32,
                int(3) as u32,
                int(2) as u32,
                int(1) as u32,
                int(0) as u32,
            );
        }
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            day_of_week: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            millis: u16,
        }
        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        unsafe extern "system" {
            fn FileTimeToSystemTime(ft: *const FileTime, st: *mut SystemTime) -> i32;
            fn SystemTimeToTzSpecificLocalTime(
                tz: *const u8,
                utc: *const SystemTime,
                local: *mut SystemTime,
            ) -> i32;
        }
        // Unix seconds → FILETIME (100 ns since 1601-01-01).
        let ticks: u64 = ((secs + 11_644_473_600) as u64) * 10_000_000;
        let ft = FileTime {
            low: ticks as u32,
            high: (ticks >> 32) as u32,
        };
        let mut utc = SystemTime::default();
        let mut local = SystemTime::default();
        // SAFETY: plain C structs, valid pointers.
        let ok = unsafe {
            FileTimeToSystemTime(&ft, &mut utc) != 0
                && SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) != 0
        };
        if ok {
            return (
                local.year as i32,
                local.month as u32,
                local.day as u32,
                local.hour as u32,
                local.minute as u32,
                local.second as u32,
            );
        }
    }
    utc_datetime(secs)
}

/// UTC calendar time of a Unix timestamp (civil-from-days, H. Hinnant).
pub fn utc_datetime(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y as i32,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `AAAAMMJJ-HHMMSS` (local time) for file names.
pub fn stamp(secs: i64) -> String {
    let (y, mo, d, h, mi, s) = local_datetime(secs);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// Date and time (local) for the interface, in the order of the language.
pub fn display_datetime(i18n: &crate::i18n::I18n, secs: i64) -> String {
    let (y, mo, d, h, mi, s) = local_datetime(secs);
    i18n.t_args(
        "date-time",
        &[
            ("y", &format!("{y:04}")),
            ("mo", &format!("{mo:02}")),
            ("d", &format!("{d:02}")),
            ("h", &format!("{h:02}")),
            ("mi", &format!("{mi:02}")),
            ("s", &format!("{s:02}")),
        ],
    )
}

/// `YYYY-MM-DD` of today (local time).
pub fn today() -> String {
    let (y, m, d, _, _, _) = local_datetime(now_secs());
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` minus `months` months (same day, clamped to 28).
pub fn date_minus_months(date: &str, months: u32) -> String {
    let mut it = date.split('-').map(|p| p.parse::<i64>().unwrap_or(0));
    let (y, m, d) = (
        it.next().unwrap_or(0),
        it.next().unwrap_or(1),
        it.next().unwrap_or(1),
    );
    let total = y * 12 + (m - 1) - months as i64;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    format!("{ny:04}-{nm:02}-{:02}", d.min(28))
}

/// Time of day (local) for the interface.
pub fn display_time(i18n: &crate::i18n::I18n, secs: i64) -> String {
    let (_, _, _, h, mi, _) = local_datetime(secs);
    i18n.t_args(
        "time-hm",
        &[("h", &format!("{h:02}")), ("mi", &format!("{mi:02}"))],
    )
}

// ---------------------------------------------------------------- rotation

/// Before overwriting `path` (R-B11-01): move the existing file to
/// `backups/<stem>_<AAAAMMJJ-HHMMSS><ext>` next to it (a `-2`, `-3`… suffix
/// on a same-second collision). With `copies == 0` the file is simply
/// overwritten. Returns the copy's path when one was made.
pub fn rotate_backup(path: &Path, copies: usize) -> std::io::Result<Option<PathBuf>> {
    if copies == 0 || !path.is_file() {
        return Ok(None);
    }
    let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
    std::fs::create_dir_all(&dir)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let base = format!("{stem}_{}", stamp(now_secs()));
    let mut candidate = dir.join(format!("{base}{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base}-{n}{ext}"));
        n += 1;
    }
    std::fs::rename(path, &candidate)?;
    Ok(Some(candidate))
}

/// Keep only the `copies` most recent copies of `path` (after a successful
/// write, R-B11-01). Copies are recognised by `<stem>_` prefix and extension.
pub fn purge_backups(path: &Path, copies: usize) -> std::io::Result<usize> {
    let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let prefix = format!("{stem}_");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(0);
    };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| n.starts_with(&prefix))
        .filter(|n| {
            Path::new(n)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default()
                == ext
        })
        .collect();
    // Names are timestamped, so lexicographic order is chronological.
    names.sort();
    let mut removed = 0;
    while names.len() > copies {
        let old = names.remove(0);
        if std::fs::remove_file(dir.join(old)).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

// ------------------------------------------------- original of an export

/// Default values of `[Output]` (E-06).
pub const DEFAULT_ORIGINAL_COPIES: usize = 5;
pub const ORIGINAL_COPIES_RANGE: std::ops::RangeInclusive<usize> = 1..=50;

/// The copy of `path` in `backups/` next to it, `<stem>_<AAAAMMJJ-HHMMSS>
/// <ext>` (`-2`, `-3`… on a same-second collision): the file is copied, not
/// moved — the export may still read it (R-E06-01).
pub fn keep_original(path: &Path) -> std::io::Result<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
    std::fs::create_dir_all(&dir)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let base = format!("{stem}_{}", stamp(now_secs()));
    let mut candidate = dir.join(format!("{base}{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{base}-{n}{ext}"));
        n += 1;
    }
    // Through a temporary name: a copy cut short never looks like a copy.
    let tmp = candidate.with_extension("part");
    std::fs::copy(path, &tmp)?;
    std::fs::rename(&tmp, &candidate)?;
    Ok(candidate)
}

/// The copies of `path` in its `backups/` folder, oldest first: exactly
/// `<stem>_<8 digits>-<6 digits>[-n]<ext>`, so « MonMod_Patch.esp » is not
/// taken for a copy of « MonMod.esp ».
pub fn original_copies(path: &Path) -> Vec<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new(".")).join("backups");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()))
        .unwrap_or_default();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<((String, u32), PathBuf)> = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !p.is_file() || name.len() < ext.len() {
            continue;
        }
        let (body, tail) = name.split_at(name.len() - ext.len());
        if tail.to_lowercase() != ext {
            continue;
        }
        let Some(rest) = body.strip_prefix(stem).and_then(|r| r.strip_prefix('_')) else {
            continue;
        };
        let (date, n) = match rest.get(15..) {
            Some("") => (rest, 1),
            Some(t) => match t.strip_prefix('-').and_then(|d| d.parse::<u32>().ok()) {
                Some(n) => (&rest[..15], n),
                None => continue,
            },
            None => continue,
        };
        let ok = date.len() == 15
            && date
                .char_indices()
                .all(|(i, c)| if i == 8 { c == '-' } else { c.is_ascii_digit() });
        if ok {
            found.push(((date.to_string(), n), p));
        }
    }
    found.sort();
    found.into_iter().map(|(_, p)| p).collect()
}

/// Keep the `copies` most recent copies of `path` (R-E06-02), after a
/// successful write. Returns the number removed.
pub fn purge_originals(path: &Path, copies: usize) -> usize {
    let list = original_copies(path);
    let extra = list.len().saturating_sub(copies);
    list.iter()
        .take(extra)
        .filter(|p| std::fs::remove_file(p).is_ok())
        .count()
}

/// Rename a mod's dictionary with its companions (B-12): its `.ximhist`
/// and its timestamped copies in `backups/`. Refused when `new` exists.
pub fn rename_with_companions(old: &Path, new: &Path) -> std::io::Result<()> {
    if new.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            new.display().to_string(),
        ));
    }
    let copies = original_copies(old);
    std::fs::rename(old, new)?;
    let (oh, nh) = (old.with_extension("ximhist"), new.with_extension("ximhist"));
    if oh.is_file() && !nh.exists() {
        std::fs::rename(&oh, &nh)?;
    }
    let (Some(os), Some(ns)) = (
        old.file_stem().and_then(|s| s.to_str()),
        new.file_stem().and_then(|s| s.to_str()),
    ) else {
        return Ok(());
    };
    for c in copies {
        if let Some(name) = c.file_name().and_then(|n| n.to_str())
            && let Some(rest) = name.strip_prefix(os)
        {
            let _ = std::fs::rename(&c, c.with_file_name(format!("{ns}{rest}")));
        }
    }
    Ok(())
}

/// Why an export did not write.
#[derive(Debug)]
pub enum ExportWriteError {
    /// The original could not be kept: nothing was written (R-E06-01).
    Keep(std::io::Error),
    Write(std::io::Error),
}

/// Before an export writes `path` itself (a text file): the copy, when the
/// file exists and `keep` or `forced`.
pub fn keep_before_export(
    path: &Path,
    keep: bool,
    forced: bool,
) -> std::io::Result<Option<PathBuf>> {
    if (keep || forced) && path.is_file() {
        keep_original(path).map(Some)
    } else {
        Ok(None)
    }
}

/// Whether two paths name the same file (the open plugin overwritten).
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Write an export output (R-E06-01/02/03): an existing file is copied to
/// `backups/` first when `keep` (or `forced`: the open plugin itself), the
/// write happens only when the copy succeeded, then the copies beyond
/// `copies` are removed. Returns the copy made, if any.
pub fn write_export(
    path: &Path,
    bytes: &[u8],
    keep: bool,
    forced: bool,
    copies: usize,
) -> Result<Option<PathBuf>, ExportWriteError> {
    let kept = keep_before_export(path, keep, forced).map_err(ExportWriteError::Keep)?;
    std::fs::write(path, bytes).map_err(ExportWriteError::Write)?;
    if kept.is_some() {
        purge_originals(path, copies.max(1));
    }
    Ok(kept)
}

// ---------------------------------------------------------------- autosave

/// Companion of a rescue file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    /// The document's own file.
    pub source: PathBuf,
    /// Unix seconds of the save.
    pub saved: i64,
    /// Lines carrying a translation at the time of the save.
    pub modified: usize,
    /// `size:<bytes>:mtime:<secs>` of the source file (cheap change check).
    pub fingerprint: String,
    /// Game, languages, output mode, personal dictionary (R-E12-07).
    pub settings: DocSettings,
    /// Set when the user chose « Ignorer » at a resume prompt: the files stay
    /// (30 days) but are not offered again.
    #[serde(default)]
    pub ignored: bool,
}

impl Manifest {
    pub fn new(source: &Path, settings: &DocSettings, modified: usize) -> Self {
        Manifest {
            format: MANIFEST_FORMAT,
            source: source.to_path_buf(),
            saved: now_secs(),
            modified,
            fingerprint: fingerprint(source),
            settings: settings.clone(),
            ignored: false,
        }
    }

    pub fn file_name(&self) -> String {
        self.source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string()
    }

    /// Whether the source file changed since the save (R-B11-05).
    pub fn source_changed(&self) -> bool {
        self.source.is_file() && fingerprint(&self.source) != self.fingerprint
    }
}

/// `size:<bytes>:mtime:<secs>` of a file (empty when unreadable).
pub fn fingerprint(path: &Path) -> String {
    match crate::slow_io("fingerprint", || std::fs::metadata(path)) {
        Ok(m) => {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("size:{}:mtime:{mtime}", m.len())
        }
        Err(_) => String::new(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutosaveEvent {
    Done {
        source: PathBuf,
        at: i64,
        revision: u64,
    },
    Failed {
        source: PathBuf,
        error: String,
    },
}

/// The autosave scheduler and its background writers.
pub struct Autosave {
    dir: PathBuf,
    interval: Duration,
    last_tick: Instant,
    tx: Sender<AutosaveEvent>,
    rx: Receiver<AutosaveEvent>,
    inflight: HashSet<PathBuf>,
}

impl Autosave {
    pub fn new(dir: PathBuf, minutes: u32) -> Self {
        let (tx, rx) = channel();
        Autosave {
            dir,
            interval: Duration::from_secs(60 * minutes.max(1) as u64),
            last_tick: Instant::now(),
            tx,
            rx,
            inflight: HashSet::new(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn set_dir(&mut self, dir: PathBuf) {
        self.dir = dir;
    }

    pub fn set_minutes(&mut self, minutes: u32) {
        self.interval = Duration::from_secs(60 * minutes.max(1) as u64);
    }

    /// Stable key of a source path (FNV-1a 64, hex), the rescue file's stem.
    pub fn key_for(source: &Path) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in source.to_string_lossy().as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{h:016x}")
    }

    pub fn state_path(&self, source: &Path) -> PathBuf {
        self.dir.join(format!("{}.json", Self::key_for(source)))
    }

    pub fn manifest_path(&self, source: &Path) -> PathBuf {
        self.dir
            .join(format!("{}.manifest.json", Self::key_for(source)))
    }

    /// Whether the interval elapsed since the last tick (resets it).
    pub fn due(&mut self) -> bool {
        if self.last_tick.elapsed() >= self.interval {
            self.last_tick = Instant::now();
            true
        } else {
            false
        }
    }

    pub fn is_inflight(&self, source: &Path) -> bool {
        self.inflight.contains(source)
    }

    /// Write `snapshot` and its manifest from a background thread (state
    /// first, manifest last, both atomically). The outcome comes back through
    /// [`poll`](Self::poll).
    pub fn spawn(&mut self, snapshot: TranslationMemory, manifest: Manifest, revision: u64) {
        let source = manifest.source.clone();
        if !self.inflight.insert(source.clone()) {
            return;
        }
        let state = self.state_path(&source);
        let mpath = self.manifest_path(&source);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let bytes = serde_json::to_vec(&snapshot).map_err(std::io::Error::other)?;
                write_atomic(&state, &bytes)?;
                let mbytes = serde_json::to_vec_pretty(&manifest).map_err(std::io::Error::other)?;
                write_atomic(&mpath, &mbytes)
            })();
            let ev = match result {
                Ok(()) => AutosaveEvent::Done {
                    source,
                    at: manifest.saved,
                    revision,
                },
                Err(e) => AutosaveEvent::Failed {
                    source,
                    error: e.to_string(),
                },
            };
            let _ = tx.send(ev);
        });
    }

    /// Events of the finished writes.
    pub fn poll(&mut self) -> Vec<AutosaveEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            let src = match &ev {
                AutosaveEvent::Done { source, .. } | AutosaveEvent::Failed { source, .. } => {
                    source.clone()
                }
            };
            self.inflight.remove(&src);
            out.push(ev);
        }
        out
    }

    /// Remove the rescue files of a document closed normally (R-B11-04).
    pub fn discard(&self, source: &Path) -> std::io::Result<()> {
        for p in [self.manifest_path(source), self.state_path(source)] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Mark a manifest as ignored by the user (kept 30 days, not offered again).
    pub fn ignore(&self, manifest: &Manifest) -> std::io::Result<()> {
        let mut m = manifest.clone();
        m.ignored = true;
        let bytes = serde_json::to_vec_pretty(&m).map_err(std::io::Error::other)?;
        write_atomic(&self.manifest_path(&manifest.source), &bytes)
    }

    /// The manifests to offer at start, most recent first, after purging the
    /// files older than [`PURGE_AFTER_DAYS`] and enforcing the quota.
    pub fn pending(dir: &Path) -> Pending {
        let mut pending = Pending::default();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return pending;
        };
        let now = now_secs();
        let mut manifests: Vec<(Manifest, PathBuf)> = Vec::new();
        for entry in rd.flatten() {
            let p = entry.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if !name.ends_with(".manifest.json") {
                continue;
            }
            let Some(m) = std::fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<Manifest>(&t).ok())
            else {
                continue;
            };
            let state = dir.join(name.replace(".manifest.json", ".json"));
            if now - m.saved > (PURGE_AFTER_DAYS * 86_400) as i64 || !state.is_file() {
                let _ = std::fs::remove_file(&state);
                let _ = std::fs::remove_file(&p);
                pending.purged += 1;
                continue;
            }
            manifests.push((m, state));
        }
        manifests.sort_by_key(|(m, _)| std::cmp::Reverse(m.saved));
        // Quota: drop the oldest until under the bound (R-B11-07).
        let mut total: u64 = manifests
            .iter()
            .map(|(_, s)| std::fs::metadata(s).map(|m| m.len()).unwrap_or(0))
            .sum();
        while total > QUOTA_BYTES && manifests.len() > 1 {
            let (m, state) = manifests.pop().unwrap();
            total = total.saturating_sub(std::fs::metadata(&state).map(|x| x.len()).unwrap_or(0));
            let _ = std::fs::remove_file(&state);
            let _ = std::fs::remove_file(
                dir.join(format!("{}.manifest.json", Self::key_for(&m.source))),
            );
            pending.quota_purged = true;
        }
        pending.items = manifests.into_iter().filter(|(m, _)| !m.ignored).collect();
        pending
    }

    pub fn load_state(json: &Path) -> anyhow::Result<TranslationMemory> {
        crate::tm::load_json_or_dsd(&std::fs::read_to_string(json)?)
    }
}

/// What [`Autosave::pending`] found.
#[derive(Debug, Default)]
pub struct Pending {
    /// `(manifest, state file)`, most recent first, ignored ones left out.
    pub items: Vec<(Manifest, PathBuf)>,
    pub purged: usize,
    pub quota_purged: bool,
}

// ---------------------------------------------------------------- resume

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeReport {
    pub matched: usize,
    pub unmatched: usize,
}

/// Apply a saved state onto a reloaded document (R-B11-05): each saved line
/// that carries a translation or a status is matched by identity
/// `(rec, fld, formid, index)`, then `(rec, fld, edid, index)`, then
/// `(rec, fld, source)`; its translation and status are copied over.
pub fn resume_into(doc: &mut [TmEntry], saved: &TranslationMemory) -> ResumeReport {
    use std::collections::HashMap;
    let mut by_formid: HashMap<(String, String, u32, i32), usize> = HashMap::new();
    let mut by_edid: HashMap<(String, String, String, i32), usize> = HashMap::new();
    let mut by_source: HashMap<(String, String, String), usize> = HashMap::new();
    for (i, e) in doc.iter().enumerate() {
        if e.formid != 0 {
            by_formid
                .entry((e.rec_sig.clone(), e.field_sig.clone(), e.formid, e.index))
                .or_insert(i);
        }
        if !e.edid.is_empty() {
            by_edid
                .entry((
                    e.rec_sig.clone(),
                    e.field_sig.clone(),
                    e.edid.to_lowercase(),
                    e.index,
                ))
                .or_insert(i);
        }
        by_source
            .entry((e.rec_sig.clone(), e.field_sig.clone(), e.source.clone()))
            .or_insert(i);
    }
    let mut r = ResumeReport::default();
    for s in &saved.entries {
        if s.trans.is_empty() && s.status_sst.is_empty() {
            continue;
        }
        let hit = (s.formid != 0)
            .then(|| by_formid.get(&(s.rec_sig.clone(), s.field_sig.clone(), s.formid, s.index)))
            .flatten()
            .or_else(|| {
                (!s.edid.is_empty())
                    .then(|| {
                        by_edid.get(&(
                            s.rec_sig.clone(),
                            s.field_sig.clone(),
                            s.edid.to_lowercase(),
                            s.index,
                        ))
                    })
                    .flatten()
            })
            .or_else(|| by_source.get(&(s.rec_sig.clone(), s.field_sig.clone(), s.source.clone())));
        match hit {
            Some(&i) => {
                doc[i].trans = s.trans.clone();
                doc[i].status_sst = s.status_sst.clone();
                doc[i].status_eet = s.status_eet;
                doc[i].prov = s.prov.clone();
                doc[i].review = s.review.clone();
                r.matched += 1;
            }
            None => r.unmatched += 1,
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::OutputMode;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ximod-backup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn settings() -> DocSettings {
        DocSettings {
            game: "SkyrimSE".into(),
            source: "eng".into(),
            target: "fra".into(),
            output: Some(OutputMode::Strings),
            use_personal: false,
        }
    }

    #[test]
    fn calendar_conversions() {
        assert_eq!(utc_datetime(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(utc_datetime(1_790_000_000), (2026, 9, 21, 14, 13, 20));
        let (y, ..) = local_datetime(1_790_000_000);
        assert_eq!(y, 2026);
        assert_eq!(stamp(0).len(), 15);
        let mut i18n = crate::i18n::I18n::new();
        i18n.set_locale("fra");
        assert_eq!(display_datetime(&i18n, 1_790_000_000).len(), 19);
    }

    #[test]
    fn rotation_keeps_n_copies() {
        let d = tmp("rot");
        let f = d.join("MonMod_ENG-FRA.ximdb");
        std::fs::write(&f, "v1").unwrap();
        // Copies = 0: nothing moved.
        assert_eq!(rotate_backup(&f, 0).unwrap(), None);
        // Three successive writes with copies = 2.
        for v in ["v2", "v3", "v4"] {
            let moved = rotate_backup(&f, 2).unwrap().unwrap();
            assert!(moved.starts_with(d.join("backups")));
            assert!(
                moved
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("MonMod_ENG-FRA_")
            );
            std::fs::write(&f, v).unwrap();
            purge_backups(&f, 2).unwrap();
        }
        let n = std::fs::read_dir(d.join("backups"))
            .unwrap()
            .flatten()
            .count();
        assert_eq!(n, 2, "exactly two timestamped copies (T-B11-03)");
        // Same-second collisions get a suffix, never overwrite.
        std::fs::write(&f, "v5").unwrap();
        let a = rotate_backup(&f, 10).unwrap().unwrap();
        std::fs::write(&f, "v6").unwrap();
        let b = rotate_backup(&f, 10).unwrap().unwrap();
        assert_ne!(a, b);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn manifest_round_trip_and_fingerprint() {
        let d = tmp("manifest");
        let src = d.join("MonMod.esp");
        std::fs::write(&src, b"abc").unwrap();
        let m = Manifest::new(&src, &settings(), 214);
        assert!(m.fingerprint.starts_with("size:3:mtime:"));
        assert!(!m.source_changed());
        let text = serde_json::to_string(&m).unwrap();
        let back: Manifest = serde_json::from_str(&text).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.file_name(), "MonMod.esp");
        std::fs::write(&src, b"abcd").unwrap();
        assert!(m.source_changed());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn spawn_poll_pending_discard() {
        let d = tmp("autosave");
        let src = d.join("MonMod.esp");
        std::fs::write(&src, b"plugin").unwrap();
        let mut auto = Autosave::new(d.join("autosave"), 5);
        assert_eq!(Autosave::key_for(&src).len(), 16);
        let mut tm = TranslationMemory::default();
        tm.entries.push(TmEntry {
            source: "a".into(),
            trans: "A".into(),
            formid: 1,
            ..Default::default()
        });
        let m = Manifest::new(&src, &settings(), 1);
        auto.spawn(tm.clone(), m.clone(), 7);
        assert!(auto.is_inflight(&src));
        let mut events = Vec::new();
        for _ in 0..200 {
            events.extend(auto.poll());
            if !events.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            events,
            vec![AutosaveEvent::Done {
                source: src.clone(),
                at: m.saved,
                revision: 7
            }]
        );
        assert!(!auto.is_inflight(&src));
        // The state reads back identical; pending() offers it.
        let back = Autosave::load_state(&auto.state_path(&src)).unwrap();
        assert_eq!(back.entries, tm.entries);
        let p = Autosave::pending(auto.dir());
        assert_eq!(p.items.len(), 1);
        assert_eq!(p.items[0].0.modified, 1);
        // Ignored: kept on disk, not offered.
        auto.ignore(&m).unwrap();
        assert!(auto.manifest_path(&src).is_file());
        assert!(Autosave::pending(auto.dir()).items.is_empty());
        // Discarded: gone (R-B11-04).
        auto.discard(&src).unwrap();
        assert!(!auto.state_path(&src).exists() && !auto.manifest_path(&src).exists());
        auto.discard(&src).unwrap(); // idempotent
        // Old manifests are purged.
        let mut old = Manifest::new(&src, &settings(), 1);
        old.saved -= (PURGE_AFTER_DAYS as i64 + 1) * 86_400;
        std::fs::write(auto.state_path(&src), b"{}").unwrap();
        std::fs::write(auto.manifest_path(&src), serde_json::to_vec(&old).unwrap()).unwrap();
        let p = Autosave::pending(auto.dir());
        assert_eq!((p.items.len(), p.purged), (0, 1));
        assert!(!auto.state_path(&src).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn due_follows_interval() {
        let mut a = Autosave::new(PathBuf::from("."), 1);
        assert!(!a.due());
        a.last_tick = Instant::now() - Duration::from_secs(61);
        assert!(a.due());
        assert!(!a.due());
    }

    #[test]
    fn resume_matches_by_identity_then_edid_then_source() {
        let e = |rec: &str, formid: u32, edid: &str, src: &str, idx: i32| TmEntry {
            rec_sig: rec.into(),
            field_sig: "FULL".into(),
            formid,
            edid: edid.into(),
            source: src.into(),
            index: idx,
            ..Default::default()
        };
        let mut doc = vec![
            e("WEAP", 1, "A", "a", 1),
            e("WEAP", 2, "B", "b", 1),
            e("WEAP", 0, "", "c", 1),
            e("WEAP", 9, "Z", "z", 1),
        ];
        let mut saved = TranslationMemory::default();
        let mut s1 = e("WEAP", 1, "A", "a", 1);
        s1.trans = "par formid".into();
        crate::status::set(&mut s1, crate::status::Status::Validated);
        let mut s2 = e("WEAP", 77, "b", "b", 1); // FormID changed: EDID matches (case-insensitive)
        s2.trans = "par edid".into();
        let mut s3 = e("WEAP", 0, "", "c", 1); // only the source
        s3.trans = "par source".into();
        let mut s4 = e("WEAP", 55, "Gone", "gone", 1); // nothing
        s4.trans = "perdu".into();
        let s5 = e("WEAP", 9, "Z", "z", 1); // untranslated: skipped
        saved.entries = vec![s1, s2, s3, s4, s5];
        let r = resume_into(&mut doc, &saved);
        assert_eq!(
            r,
            ResumeReport {
                matched: 3,
                unmatched: 1
            }
        );
        assert_eq!(doc[0].trans, "par formid");
        assert_eq!(
            crate::status::derive(&doc[0]),
            crate::status::Status::Validated
        );
        assert_eq!(doc[1].trans, "par edid");
        assert_eq!(doc[2].trans, "par source");
        assert_eq!(doc[3].trans, "");
    }
}
