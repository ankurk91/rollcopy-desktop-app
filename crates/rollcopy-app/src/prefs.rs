//! App-wide preferences as a gpui global (PLAN §8a). Changing a preference
//! saves `config.toml` at once (no OK/Apply) and notifies every window that
//! observes `Prefs`, so changes apply immediately.

use std::path::PathBuf;

use gpui_kit::*;

use crate::config::{self, Config, ThemePref, UiState};
use crate::theme::ThemeChoice;

pub struct Prefs {
    pub config: Config,
    pub ui: UiState,
    config_path: PathBuf,
    ui_path: PathBuf,
    /// One-line notice for the user (e.g. a broken config file was reset).
    pub notice: Option<String>,
    /// A reset is under way: don't write anything back before the restart.
    resetting: bool,
}

impl Global for Prefs {}

impl Prefs {
    /// Loads `~/.config/rollcopy/config.toml` and `~/.local/state/rollcopy/ui.toml`.
    pub fn load() -> Self {
        let dirs = directories::ProjectDirs::from("io.github", "ankurk91", "rollcopy");
        let config_path = dirs
            .as_ref()
            .map(|d| d.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("rollcopy-config.toml"));
        let ui_path = dirs
            .as_ref()
            .map(|d| {
                d.state_dir()
                    .unwrap_or_else(|| d.data_local_dir())
                    .join("ui.toml")
            })
            .unwrap_or_else(|| PathBuf::from("rollcopy-ui.toml"));
        let (config, notice) = config::load::<Config>(&config_path);
        let (ui, _) = config::load::<UiState>(&ui_path);
        Self {
            config,
            ui,
            config_path,
            ui_path,
            notice,
            resetting: false,
        }
    }

    /// Error history database, next to `ui.toml` in the state directory.
    pub fn history_path(&self) -> PathBuf {
        self.ui_path.with_file_name("history.sqlite")
    }

    /// Resets preferences, window state, the error history and the cache
    /// (thumbnails, Photos database copies, temporary files) to a fresh start,
    /// then restarts the app. Copied files and the phone are not touched.
    pub fn reset_all_and_restart(cx: &mut App) {
        clear_cache();
        let p = cx.global_mut::<Prefs>();
        p.resetting = true;
        let history = p.history_path();
        let mut files = vec![p.config_path.clone(), p.ui_path.clone(), history.clone()];
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut f = history.as_os_str().to_owned();
            f.push(suffix);
            files.push(PathBuf::from(f));
        }
        for f in files {
            if let Err(e) = std::fs::remove_file(&f)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("reset: couldn't remove {}: {e}", f.display());
            }
        }
        cx.restart();
    }

    pub fn get(cx: &App) -> &Prefs {
        cx.global::<Prefs>()
    }

    /// Changes preferences, saves them, and notifies observers.
    pub fn update_config(cx: &mut App, f: impl FnOnce(&mut Config)) {
        cx.update_global::<Prefs, _>(|p, _| {
            if p.resetting {
                return;
            }
            f(&mut p.config);
            if let Err(e) = config::save(&p.config_path, &p.config) {
                p.notice = Some(format!("Couldn't save settings: {e}"));
            }
        });
    }

    /// Changes and saves window state (no need to notify: nothing renders it).
    pub fn update_ui(cx: &mut App, f: impl FnOnce(&mut UiState)) {
        let p = cx.global_mut::<Prefs>();
        if p.resetting {
            return;
        }
        f(&mut p.ui);
        let _ = config::save(&p.ui_path, &p.ui);
    }

    pub fn config_path(&self) -> &std::path::Path {
        &self.config_path
    }
}

impl From<ThemePref> for ThemeChoice {
    fn from(t: ThemePref) -> Self {
        match t {
            ThemePref::System => ThemeChoice::System,
            ThemePref::Light => ThemeChoice::Light,
            ThemePref::Dark => ThemeChoice::Dark,
        }
    }
}

impl From<ThemeChoice> for ThemePref {
    fn from(t: ThemeChoice) -> Self {
        match t {
            ThemeChoice::System => ThemePref::System,
            ThemeChoice::Light => ThemePref::Light,
            ThemeChoice::Dark => ThemePref::Dark,
        }
    }
}

/// Deletes `~/.cache/rollcopy` (everything in it is rebuilt on demand). Only
/// removes a directory named `rollcopy` inside the user's cache directory.
fn clear_cache() {
    let Some(dirs) = directories::ProjectDirs::from("io.github", "ankurk91", "rollcopy") else {
        return;
    };
    let cache = dirs.cache_dir();
    let base = directories::BaseDirs::new().map(|b| b.cache_dir().to_path_buf());
    let safe = cache.file_name().is_some_and(|n| n == "rollcopy")
        && base.is_some_and(|b| cache.parent() == Some(b.as_path()));
    if !safe {
        eprintln!(
            "reset: not clearing unexpected cache path {}",
            cache.display()
        );
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(cache)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("reset: couldn't clear {}: {e}", cache.display());
    }
}
