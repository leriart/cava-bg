//! Canonical filesystem layout for all runtime state.
//!
//! Every path used by the daemon, the supervisor, the renderer and the GUI is
//! derived from a single `config_path` through [`RuntimePaths`]. Previously each
//! module re-implemented its own `config_path.parent().unwrap_or("/tmp")` logic,
//! and several call sites (pid files, daemon logs, the live-colour socket,
//! presets, parallax profiles) hardcoded `$HOME/.config/cava-bg` and therefore
//! silently ignored `--config`.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// Directory name used inside the XDG config root.
const APP_DIR: &str = "cava-bg";
const CONFIG_FILE: &str = "config.toml";
const RUNTIME_OUTPUTS: &str = "runtime-outputs.json";
const COLOR_SOCKET: &str = "runtime-color-update.sock";

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `~/.config/cava-bg/config.toml`, honouring `XDG_CONFIG_HOME`.
///
/// The original implementation formatted `$HOME/.config/...` by hand, which
/// ignored `XDG_CONFIG_HOME` entirely.
pub fn default_config_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg).join(APP_DIR);
    }
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join(APP_DIR)
}

pub fn default_config_path() -> PathBuf {
    default_config_dir().join(CONFIG_FILE)
}

/// All runtime files for one configuration, derived from the config file path.
///
/// The owning directory is the single source of truth: keep the config file and
/// its pid file / log / sockets together so `--config /tmp/x.toml` works
/// completely self-contained.
#[derive(Debug, Clone)]
pub struct RuntimePaths {
    dir: PathBuf,
}

impl RuntimePaths {
    pub fn new(config_path: &Path) -> Self {
        let dir = config_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(default_config_dir);
        Self { dir }
    }

    /// Convenience for the common case of the default configuration location.
    pub fn default() -> Self {
        Self::new(&default_config_path())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `daemon.pid`, or `daemon.<output>.pid` in supervisor mode.
    pub fn pid_file(&self, output: Option<&str>) -> PathBuf {
        match output {
            Some(name) => self.dir.join(format!("daemon.{name}.pid")),
            None => self.dir.join("daemon.pid"),
        }
    }

    /// `daemon.log`, or `daemon.<output>.log` for a supervised child.
    pub fn daemon_log(&self, output: Option<&str>) -> PathBuf {
        match output {
            Some(name) => self.dir.join(format!("daemon.{name}.log")),
            None => self.dir.join("daemon.log"),
        }
    }

    /// Live output geometry written by the renderer/supervisor, read by the CLI.
    pub fn runtime_outputs(&self) -> PathBuf {
        self.dir.join(RUNTIME_OUTPUTS)
    }

    /// Unix datagram socket used by the GUI to push colours to a running daemon.
    pub fn color_socket(&self) -> PathBuf {
        self.dir.join(COLOR_SOCKET)
    }

    pub fn presets_dir(&self) -> PathBuf {
        self.dir.join("presets")
    }

    pub fn parallax_dir(&self) -> PathBuf {
        self.dir.join("parallax")
    }

    pub fn legacy_pid_files(&self) -> Vec<PathBuf> {
        vec![
            self.dir.join("cava-bg.pid"),
            PathBuf::from("/tmp/cava-bg.pid"),
        ]
    }

    pub fn ensure_dir(&self) -> io::Result<()> {
        fs::create_dir_all(&self.dir)
    }
}

/// Write `contents` to `path` atomically (temp file in the same directory, fsync,
/// rename) so concurrent readers never observe a truncated file.
///
/// `runtime-outputs.json` is written by the renderer, the supervisor and read by
/// the CLI and the GUI at the same time; a plain `fs::write` could be observed
/// mid-write.
pub fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;

    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".to_string());
    let tmp = dir.join(format!(".{}.{}.{}.tmp", file_name, std::process::id(), n));

    let result = (|| -> io::Result<()> {
        {
            let mut file = fs::File::create(&tmp)?;
            use std::io::Write;
            file.write_all(contents)?;
            file.sync_all()?;
        }
        fs::rename(&tmp, path)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// `true` when `path` exists and its mtime is strictly newer than `since`.
///
/// Used by the config hot-reload and supervisor watchers; returning `false` on
/// any error keeps a transient failure from being treated as a change.
pub fn modified_since(path: &Path, since: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|m| m > since)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_everything_from_config_directory() {
        let paths = RuntimePaths::new(Path::new("/tmp/custom/config.toml"));
        assert_eq!(paths.dir(), Path::new("/tmp/custom"));
        assert_eq!(paths.pid_file(None), Path::new("/tmp/custom/daemon.pid"));
        assert_eq!(
            paths.pid_file(Some("DP-1")),
            Path::new("/tmp/custom/daemon.DP-1.pid")
        );
        assert_eq!(
            paths.daemon_log(Some("DP-1")),
            Path::new("/tmp/custom/daemon.DP-1.log")
        );
        assert_eq!(
            paths.runtime_outputs(),
            Path::new("/tmp/custom/runtime-outputs.json")
        );
        assert_eq!(
            paths.color_socket(),
            Path::new("/tmp/custom/runtime-color-update.sock")
        );
        assert_eq!(paths.presets_dir(), Path::new("/tmp/custom/presets"));
        assert_eq!(paths.parallax_dir(), Path::new("/tmp/custom/parallax"));
    }

    #[test]
    fn custom_config_path_matches_gui_socket() {
        // Regression: the daemon bound the socket next to the config file while
        // the GUI looked for it under ~/.config/cava-bg, so the live preview was
        // silently dead for any `--config` outside the default directory.
        let paths = RuntimePaths::new(Path::new("/opt/shared/cava.toml"));
        assert_eq!(paths.color_socket(), paths.color_socket());
        assert!(paths.color_socket().starts_with("/opt/shared"));
    }

    #[test]
    fn two_configs_never_share_runtime_state() {
        // Regression: `pid_file_path` used to always return the *default*
        // config dir regardless of `--config`, so two installed builds pointed
        // at different configs would fight over one pid file -- the second
        // refusing to start and `off` killing the wrong daemon.
        let prod = RuntimePaths::new(Path::new("/home/u/.config/cava-bg/config.toml"));
        let dev = RuntimePaths::new(Path::new("/home/u/.local/share/cava-bg-dev/config.toml"));

        assert_ne!(prod.pid_file(None), dev.pid_file(None));
        assert_ne!(prod.daemon_log(None), dev.daemon_log(None));
        assert_ne!(prod.color_socket(), dev.color_socket());
        assert_ne!(prod.runtime_outputs(), dev.runtime_outputs());
        assert_ne!(prod.presets_dir(), dev.presets_dir());

        // Same for the per-output variants the supervisor uses.
        assert_ne!(prod.pid_file(Some("DP-1")), dev.pid_file(Some("DP-1")));
    }

    #[test]
    fn bare_filename_falls_back_to_default_dir() {
        let paths = RuntimePaths::new(Path::new("config.toml"));
        assert_eq!(paths.dir(), default_config_dir());
    }

    #[test]
    fn default_paths_honour_xdg_config_home() {
        // SAFETY: single-threaded test body, env restored immediately.
        unsafe {
            env::set_var("XDG_CONFIG_HOME", "/xdg");
        }
        assert_eq!(
            default_config_path(),
            PathBuf::from("/xdg/cava-bg/config.toml")
        );
        unsafe {
            env::remove_var("XDG_CONFIG_HOME");
        }
    }

    #[test]
    fn atomic_write_replaces_contents_without_truncation_window() {
        let dir = std::env::temp_dir().join(format!("cava-bg-atomic-{n}", n = std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("state.json");

        atomic_write(&target, b"{\"a\":1}").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "{\"a\":1}");

        atomic_write(&target, b"{\"a\":2}").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "{\"a\":2}");

        // No leftover temp files.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "state.json")
            .collect();
        assert!(leftovers.is_empty(), "unexpected temp files: {leftovers:?}");

        fs::remove_dir_all(&dir).ok();
    }
}
