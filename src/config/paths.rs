//! Where the application writes for the user (E-05).
//!
//! Two modes exist. **Portable**: a `Config.ini` sits next to the executable and
//! everything the user produces (configuration, personal dictionaries, edited
//! data files, autosaves, logs) lives beside it — the historical layout, kept
//! as-is for existing installations. **User**: nothing is written next to the
//! executable (which may be under `Program Files`); the per-user folder of the
//! operating system is used instead (R-E05-01):
//!
//! | OS      | Folder                                                     |
//! |---------|------------------------------------------------------------|
//! | Windows | `%APPDATA%\XIMOD Translator`                               |
//! | Linux   | `$XDG_CONFIG_HOME/ximod-translator` (`~/.config/…`)        |
//! | macOS   | `~/Library/Application Support/XIMOD Translator`           |
//!
//! The mode is decided once at startup by [`init`] from the presence of
//! `Config.ini` next to the executable, and never changes while the program
//! runs: switching from the Settings copies everything to the other location
//! and lays down (or sets aside) the marker file, the change taking effect at
//! the next start (R-E05-03).
//!
//! Every write of a configuration or data file goes through [`write_atomic`]
//! (temporary file in the same folder, then rename), so a crash or a full disk
//! never leaves a truncated file behind.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Folder name under the per-user configuration root (Windows / macOS).
const USER_FOLDER_TITLE: &str = "XIMOD Translator";
/// Folder name under `$XDG_CONFIG_HOME` (Linux).
const USER_FOLDER_XDG: &str = "ximod-translator";
/// Name of the configuration file (and of the portable-mode marker).
pub const CONFIG_FILE: &str = "Config.ini";
/// Name given to the former portable `Config.ini` after switching to user mode
/// (R-E05-03): it stops acting as the marker but stays recoverable.
pub const PORTABLE_OFF_FILE: &str = "Config.ini.portable-off";
/// Environment variable letting tests (and support) simulate the executable's
/// folder without moving the binary.
pub const EXE_DIR_ENV: &str = "XIMOD_EXE_DIR";

/// Where the configuration lives (R-E05-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigMode {
    /// `Config.ini` next to the executable; everything is written there.
    Portable,
    /// Per-user folder of the operating system.
    User,
}

impl ConfigMode {
    /// Value written to `[General] ConfigMode` (informative).
    pub fn as_str(self) -> &'static str {
        match self {
            ConfigMode::Portable => "portable",
            ConfigMode::User => "user",
        }
    }
}

/// The resolved folders. `exe_dir` holds the shipped, read-only assets; the
/// other folders are the writable ones (R-E05-05).
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub mode: ConfigMode,
    /// Folder of the executable (shipped assets, read-only in user mode).
    pub exe_dir: PathBuf,
    /// The per-user root this resolution was made with (`None`: the OS gives
    /// no configuration folder, user mode unavailable).
    pub user_root: Option<PathBuf>,
    /// Folder of `Config.ini`.
    pub config_dir: PathBuf,
    /// `<config>/data`: user data files (`fields/`, `bases/{jeux,mods}` in user mode).
    pub data_dir: PathBuf,
    /// `<config>/autosave` (B-11; overridable by `[Backup] AutosaveDir`).
    pub autosave_dir: PathBuf,
    /// `<config>/dict`: personal spell-checker dictionaries.
    pub dict_dir: PathBuf,
    /// `<config>/logs`.
    pub logs_dir: PathBuf,
    /// True when the configuration folder cannot be written (R-E05-06): the
    /// application still starts, with a read-only configuration, and says so.
    pub read_only: bool,
}

/// Why a mode switch was refused (R-E05-03).
#[derive(Debug)]
pub enum MigrateError {
    /// The destination folder cannot be written (typically `Program Files`).
    NotWritable(PathBuf),
    /// A copy failed part-way; nothing was removed from the source.
    Io(PathBuf, io::Error),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::NotWritable(p) => {
                write!(
                    f,
                    "{}",
                    crate::l10n!("err-not-writable", path = p.display())
                )
            }
            MigrateError::Io(p, e) => {
                write!(
                    f,
                    "{}",
                    crate::l10n!("err-path-error", path = p.display(), error = e)
                )
            }
        }
    }
}

impl std::error::Error for MigrateError {}

impl AppPaths {
    /// Folder of the executable: `XIMOD_EXE_DIR` when set (tests, support),
    /// else the parent of the running binary, else the working directory.
    pub fn exe_dir() -> PathBuf {
        if let Ok(v) = std::env::var(EXE_DIR_ENV)
            && !v.trim().is_empty()
        {
            return PathBuf::from(v.trim());
        }
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// Per-user root (R-E05-01), `None` when the OS gives no configuration folder.
    pub fn user_root() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        Some(base.join(if cfg!(target_os = "linux") {
            USER_FOLDER_XDG
        } else {
            USER_FOLDER_TITLE
        }))
    }

    /// Decide the mode and lay out the folders (R-E05-01): `Config.ini` next
    /// to the executable ⇒ portable, else per-user. Nothing is created here.
    pub fn resolve() -> AppPaths {
        Self::resolve_in(Self::exe_dir(), Self::user_root())
    }

    /// [`resolve`](Self::resolve) with explicit roots (testable).
    pub fn resolve_in(exe_dir: PathBuf, user_root: Option<PathBuf>) -> AppPaths {
        let portable = exe_dir.join(CONFIG_FILE).is_file();
        let (mode, config_dir) = match (portable, &user_root) {
            (true, _) => (ConfigMode::Portable, exe_dir.clone()),
            (false, Some(root)) => (ConfigMode::User, root.clone()),
            // No per-user folder at all (exotic): stay next to the executable.
            (false, None) => (ConfigMode::Portable, exe_dir.clone()),
        };
        Self::layout(mode, exe_dir, user_root, config_dir)
    }

    fn layout(
        mode: ConfigMode,
        exe_dir: PathBuf,
        user_root: Option<PathBuf>,
        config_dir: PathBuf,
    ) -> AppPaths {
        AppPaths {
            mode,
            exe_dir,
            user_root,
            data_dir: config_dir.join("data"),
            autosave_dir: config_dir.join("autosave"),
            dict_dir: config_dir.join("dict"),
            logs_dir: config_dir.join("logs"),
            config_dir,
            read_only: false,
        }
    }

    /// Full path of `Config.ini`.
    pub fn config_path(&self) -> PathBuf {
        self.config_dir.join(CONFIG_FILE)
    }

    /// The four writable sub-folders, in a fixed order.
    fn sub_dirs(&self) -> [&PathBuf; 4] {
        [
            &self.data_dir,
            &self.autosave_dir,
            &self.dict_dir,
            &self.logs_dir,
        ]
    }

    /// Create the configuration folder and its sub-folders (R-E05-01, R-E05-05).
    pub fn ensure_dirs(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.config_dir)?;
        for d in self.sub_dirs() {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    /// Where the other mode would put its configuration, if that mode is
    /// available at all (user mode needs a per-user root).
    pub fn config_dir_for(&self, mode: ConfigMode) -> Option<PathBuf> {
        match mode {
            ConfigMode::Portable => Some(self.exe_dir.clone()),
            ConfigMode::User => self.user_root.clone(),
        }
    }

    /// Switch modes (R-E05-03): copy `Config.ini`, the personal dictionaries,
    /// the user data files and the autosaves to the other location, then lay
    /// down the marker (portable) or set it aside (user). Nothing is deleted
    /// from the current location. Returns the new configuration folder; the
    /// change takes effect at the next start.
    pub fn migrate_to(&self, target: ConfigMode) -> Result<PathBuf, MigrateError> {
        let dest_dir = match self.config_dir_for(target) {
            Some(d) => d,
            None => return Err(MigrateError::NotWritable(PathBuf::from("?"))),
        };
        if target == self.mode && dest_dir == self.config_dir {
            return Ok(dest_dir);
        }
        let dest = Self::layout(
            target,
            self.exe_dir.clone(),
            self.user_root.clone(),
            dest_dir.clone(),
        );
        if !is_writable_dir(&dest.config_dir) {
            return Err(MigrateError::NotWritable(dest.config_dir));
        }
        dest.ensure_dirs()
            .map_err(|e| MigrateError::Io(dest.config_dir.clone(), e))?;

        // Configuration, with the informative mode key set for the destination.
        let src_cfg = self.config_path();
        if src_cfg.is_file() {
            let text = std::fs::read_to_string(&src_cfg)
                .map_err(|e| MigrateError::Io(src_cfg.clone(), e))?;
            let text = set_config_mode_line(&text, target);
            write_atomic(&dest.config_path(), text.as_bytes())
                .map_err(|e| MigrateError::Io(dest.config_path(), e))?;
        }
        // Personal dictionaries, user data, autosaves (logs are not worth carrying).
        for (from, to) in [
            (&self.dict_dir, &dest.dict_dir),
            (&self.data_dir, &dest.data_dir),
            (&self.autosave_dir, &dest.autosave_dir),
        ] {
            copy_dir_all(from, to).map_err(|e| MigrateError::Io(from.clone(), e))?;
        }
        // The marker: `Config.ini` next to the executable means portable. Going
        // to user mode sets the portable one aside (recoverable), going to
        // portable the copy above IS the marker.
        if target == ConfigMode::User {
            let marker = self.exe_dir.join(CONFIG_FILE);
            if marker.is_file() {
                let off = self.exe_dir.join(PORTABLE_OFF_FILE);
                std::fs::rename(&marker, &off).map_err(|e| MigrateError::Io(marker, e))?;
            }
        }
        Ok(dest.config_dir)
    }

    /// One-time copy of files that older versions wrote next to the executable
    /// (R-E05-04): `personal_<code>.dic` → `dict/`, `ximod_startup.log` →
    /// `logs/`. A file already present at the new location is left alone; the
    /// old one is never removed.
    pub fn migrate_legacy_files(&self) {
        let Ok(rd) = std::fs::read_dir(&self.exe_dir) else {
            return;
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let target = if name.starts_with("personal_") && name.ends_with(".dic") {
                self.dict_dir.join(&name)
            } else if name == "ximod_startup.log" {
                self.logs_dir.join(&name)
            } else {
                continue;
            };
            if target.exists() || target == path {
                continue;
            }
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::copy(&path, &target) {
                tracing::warn!("legacy file {} not copied: {e}", path.display());
            }
        }
    }
}

/// Write `bytes` to `path` through a temporary file in the same folder that is
/// then renamed over the target: readers see either the old content or the
/// whole new one, never a truncated file. On Windows `std::fs::rename` replaces
/// an existing target (MoveFileEx), as on Unix.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{stem}.{}.tmp", std::process::id()));
    let result = (|| {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Whether the folder can be written to (creating it if it does not exist yet):
/// a probe file is created and removed.
pub fn is_writable_dir(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(format!(".ximod-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Recursive copy, overwriting files that already exist at the destination
/// (the user asked for the current state to be carried over). A missing
/// source is not an error.
fn copy_dir_all(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir_all(&src, &dst)?;
        } else if src.is_file() {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// Set (or add under `[General]`) the informative `ConfigMode=` line.
fn set_config_mode_line(text: &str, mode: ConfigMode) -> String {
    let line = format!("ConfigMode={}", mode.as_str());
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    for l in text.lines() {
        if l.trim_start().starts_with("ConfigMode=") {
            if !done {
                out.push(line.clone());
                done = true;
            }
        } else {
            out.push(l.to_string());
        }
    }
    if !done {
        if let Some(pos) = out.iter().position(|l| l.trim() == "[General]") {
            out.insert(pos + 1, line);
        } else {
            out.push(line);
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

static PATHS: OnceLock<AppPaths> = OnceLock::new();

/// Resolve the folders once, create them, carry over the legacy files and
/// detect a read-only configuration folder (R-E05-06). Called at the very
/// start of `main`, before the configuration is loaded; later calls return
/// the same value.
pub fn init() -> &'static AppPaths {
    PATHS.get_or_init(|| {
        let mut p = AppPaths::resolve();
        if p.ensure_dirs().is_err() || !is_writable_dir(&p.config_dir) {
            p.read_only = true;
        } else {
            p.migrate_legacy_files();
        }
        p
    })
}

/// The folders of the command line (D-06, R-D06-01): resolved as the
/// interface resolves them, or with `config_dir` as the folder of
/// `Config.ini` (`--config`); nothing is created, carried over or written.
/// A portable folder given to `--config` is taken through
/// [`EXE_DIR_ENV`] before this call.
pub fn init_read_only(config_dir: Option<&Path>) -> &'static AppPaths {
    PATHS.get_or_init(|| match config_dir {
        None => AppPaths::resolve(),
        Some(dir) => {
            let exe = AppPaths::exe_dir();
            let mode = if exe == dir {
                ConfigMode::Portable
            } else {
                ConfigMode::User
            };
            AppPaths::layout(mode, exe, AppPaths::user_root(), dir.to_path_buf())
        }
    })
}

/// The resolved folders (initialising them if `init` was not called yet, as in
/// tests — then nothing is created or migrated).
pub fn get() -> &'static AppPaths {
    PATHS.get_or_init(AppPaths::resolve)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ximod-paths-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn config_next_to_exe_means_portable() {
        let exe = temp_dir("portable");
        std::fs::write(exe.join(CONFIG_FILE), "[General]\n").unwrap();
        let user = exe.join("user-root");
        let p = AppPaths::resolve_in(exe.clone(), Some(user.clone()));
        assert_eq!(p.mode, ConfigMode::Portable);
        assert_eq!(p.config_dir, exe);
        assert_eq!(p.dict_dir, exe.join("dict"));
        assert_eq!(p.config_path(), exe.join(CONFIG_FILE));
        assert!(!user.exists(), "user mode folder must not be touched");
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn no_config_next_to_exe_means_user() {
        let exe = temp_dir("user");
        let user = exe.join("user-root");
        let p = AppPaths::resolve_in(exe.clone(), Some(user.clone()));
        assert_eq!(p.mode, ConfigMode::User);
        assert_eq!(p.config_dir, user);
        assert_eq!(p.data_dir, user.join("data"));
        assert_eq!(p.logs_dir, user.join("logs"));
        // Without any per-user root, fall back next to the executable.
        let q = AppPaths::resolve_in(exe.clone(), None);
        assert_eq!(q.mode, ConfigMode::Portable);
        assert_eq!(q.config_dir, exe);
        let _ = std::fs::remove_dir_all(&exe);
    }

    #[test]
    fn exe_dir_env_override() {
        let d = temp_dir("env");
        // SAFETY: tests of this module run on one thread each; the variable is
        // restored right after.
        unsafe { std::env::set_var(EXE_DIR_ENV, &d) };
        assert_eq!(AppPaths::exe_dir(), d);
        unsafe { std::env::remove_var(EXE_DIR_ENV) };
        assert_ne!(AppPaths::exe_dir(), d);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn ensure_dirs_creates_the_four_sub_folders() {
        let root = temp_dir("ensure");
        let p = AppPaths::resolve_in(root.join("exe"), Some(root.join("cfg")));
        p.ensure_dirs().unwrap();
        for d in [&p.data_dir, &p.autosave_dir, &p.dict_dir, &p.logs_dir] {
            assert!(d.is_dir(), "{}", d.display());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrate_portable_to_user_copies_and_sets_marker_aside() {
        let root = temp_dir("p2u");
        let exe = root.join("exe");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join(CONFIG_FILE), "[General]\nLocale=fra\n").unwrap();
        let user = root.join("cfg");
        let p = AppPaths::resolve_in(exe.clone(), Some(user.clone()));
        assert_eq!(p.mode, ConfigMode::Portable);
        p.ensure_dirs().unwrap();
        std::fs::write(p.dict_dir.join("personal_fra.dic"), "mot\n").unwrap();
        std::fs::create_dir_all(p.data_dir.join("fields")).unwrap();
        std::fs::write(p.data_dir.join("fields").join("Skyrim.json"), "{}").unwrap();
        std::fs::write(p.autosave_dir.join("a.json"), "{}").unwrap();

        let new_dir = p.migrate_to(ConfigMode::User).unwrap();
        assert_eq!(new_dir, user);
        let cfg = std::fs::read_to_string(user.join(CONFIG_FILE)).unwrap();
        assert!(cfg.contains("Locale=fra"));
        assert!(cfg.contains("ConfigMode=user"));
        assert_eq!(
            std::fs::read_to_string(user.join("dict").join("personal_fra.dic")).unwrap(),
            "mot\n"
        );
        assert!(
            user.join("data")
                .join("fields")
                .join("Skyrim.json")
                .is_file()
        );
        assert!(user.join("autosave").join("a.json").is_file());
        // Marker set aside, sources untouched.
        assert!(!exe.join(CONFIG_FILE).exists());
        assert!(exe.join(PORTABLE_OFF_FILE).is_file());
        assert!(p.dict_dir.join("personal_fra.dic").is_file());
        // Next start resolves to user mode.
        let q = AppPaths::resolve_in(exe.clone(), Some(user.clone()));
        assert_eq!(q.mode, ConfigMode::User);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn migrate_user_to_portable_creates_marker() {
        let root = temp_dir("u2p");
        let exe = root.join("exe");
        let user = root.join("cfg");
        let p = AppPaths::resolve_in(exe.clone(), Some(user.clone()));
        assert_eq!(p.mode, ConfigMode::User);
        p.ensure_dirs().unwrap();
        std::fs::write(p.config_path(), "[General]\nConfigMode=user\nLocale=eng\n").unwrap();
        let new_dir = p.migrate_to(ConfigMode::Portable).unwrap();
        assert_eq!(new_dir, exe);
        let cfg = std::fs::read_to_string(exe.join(CONFIG_FILE)).unwrap();
        assert!(cfg.contains("ConfigMode=portable"));
        assert!(cfg.contains("Locale=eng"));
        assert_eq!(cfg.matches("ConfigMode=").count(), 1);
        assert!(exe.join("dict").is_dir());
        let q = AppPaths::resolve_in(exe.clone(), Some(user.clone()));
        assert_eq!(q.mode, ConfigMode::Portable);
        // Same mode again: no-op.
        assert_eq!(q.migrate_to(ConfigMode::Portable).unwrap(), exe);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn migrate_to_portable_refused_when_exe_dir_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        // Root ignores permissions: the check is only meaningful otherwise.
        if unsafe { libc_geteuid() } == 0 {
            return;
        }
        let root = temp_dir("ro");
        let exe = root.join("exe");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o555)).unwrap();
        let user = root.join("cfg");
        let p = AppPaths::resolve_in(exe.clone(), Some(user));
        p.ensure_dirs().unwrap();
        match p.migrate_to(ConfigMode::Portable) {
            Err(MigrateError::NotWritable(d)) => assert_eq!(d, exe),
            other => panic!("expected NotWritable, got {other:?}"),
        }
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    unsafe fn libc_geteuid() -> u32 {
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        unsafe { geteuid() }
    }

    #[test]
    fn legacy_files_copied_once() {
        let root = temp_dir("legacy");
        let exe = root.join("exe");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join("personal_fra.dic"), "ancien\n").unwrap();
        std::fs::write(exe.join("ximod_startup.log"), "log\n").unwrap();
        std::fs::write(exe.join("other.txt"), "x").unwrap();
        let p = AppPaths::resolve_in(exe.clone(), Some(root.join("cfg")));
        p.ensure_dirs().unwrap();
        p.migrate_legacy_files();
        assert_eq!(
            std::fs::read_to_string(p.dict_dir.join("personal_fra.dic")).unwrap(),
            "ancien\n"
        );
        assert!(p.logs_dir.join("ximod_startup.log").is_file());
        assert!(!p.dict_dir.join("other.txt").exists());
        assert!(exe.join("personal_fra.dic").is_file(), "old file kept");
        // Newer content at the destination is not overwritten.
        std::fs::write(p.dict_dir.join("personal_fra.dic"), "nouveau\n").unwrap();
        p.migrate_legacy_files();
        assert_eq!(
            std::fs::read_to_string(p.dict_dir.join("personal_fra.dic")).unwrap(),
            "nouveau\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp() {
        let d = temp_dir("atomic");
        let f = d.join("sub").join("Config.ini");
        write_atomic(&f, b"one").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"one");
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(f.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn write_atomic_failure_keeps_old_content() {
        let d = temp_dir("atomic-fail");
        let f = d.join("Config.ini");
        write_atomic(&f, b"keep").unwrap();
        // A directory in the way of the rename makes it fail.
        let blocked = d.join("blocked");
        std::fs::create_dir_all(blocked.join("child")).unwrap();
        assert!(write_atomic(&blocked, b"x").is_err());
        assert_eq!(std::fs::read(&f).unwrap(), b"keep");
        let leftovers: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temporary file removed on failure");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn config_mode_line_set_or_added() {
        let s = set_config_mode_line("[General]\nLocale=fra\n", ConfigMode::User);
        assert_eq!(s, "[General]\nConfigMode=user\nLocale=fra\n");
        let s = set_config_mode_line(&s, ConfigMode::Portable);
        assert_eq!(s, "[General]\nConfigMode=portable\nLocale=fra\n");
        let s = set_config_mode_line("Locale=fra\n", ConfigMode::User);
        assert_eq!(s, "Locale=fra\nConfigMode=user\n");
    }
}
