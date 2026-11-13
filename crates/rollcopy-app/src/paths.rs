//! Where Rollcopy keeps its files, worked out once (audit MNT-07).

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct AppDirs {
    /// `~/.config/rollcopy`: config.toml.
    pub config: PathBuf,
    /// `~/.local/state/rollcopy`: window state, failure history, log, lock.
    pub state: PathBuf,
    /// `~/.cache/rollcopy`: thumbnails and session-only files.
    pub cache: PathBuf,
}

impl AppDirs {
    pub fn config_file(&self) -> PathBuf {
        self.config.join("config.toml")
    }

    pub fn ui_file(&self) -> PathBuf {
        self.state.join("ui.toml")
    }

    pub fn history(&self) -> PathBuf {
        self.state.join("history.sqlite")
    }

    pub fn log(&self) -> PathBuf {
        self.state.join("rollcopy.log")
    }
}

/// The app's directories (XDG locations; a temp folder if there is no home).
pub fn app_dirs() -> &'static AppDirs {
    static DIRS: OnceLock<AppDirs> = OnceLock::new();
    DIRS.get_or_init(
        || match directories::ProjectDirs::from("io.github", "ankurk91", "rollcopy") {
            Some(d) => AppDirs {
                config: d.config_dir().to_path_buf(),
                state: d
                    .state_dir()
                    .unwrap_or_else(|| d.data_local_dir())
                    .to_path_buf(),
                cache: d.cache_dir().to_path_buf(),
            },
            None => {
                let tmp = std::env::temp_dir().join("rollcopy");
                AppDirs {
                    config: tmp.clone(),
                    state: tmp.clone(),
                    cache: tmp,
                }
            }
        },
    )
}
