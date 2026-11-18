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
                let tmp = private_temp_dir();
                AppDirs {
                    config: tmp.clone(),
                    state: tmp.clone(),
                    cache: tmp,
                }
            }
        },
    )
}

/// Fallback folder with no home: `/tmp/rollcopy-<uid>`, so another user
/// can't have made it first (a shared `/tmp/rollcopy` could be pre-created
/// or a symlink; review S5). If it exists and isn't ours, a name for this
/// process is used instead. It's made owner-only when first used.
fn private_temp_dir() -> PathBuf {
    let tmp = std::env::temp_dir();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        // Our uid: the owner of our own /proc entry.
        let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).ok();
        if let Some(uid) = uid {
            let dir = tmp.join(format!("rollcopy-{uid}"));
            match std::fs::symlink_metadata(&dir) {
                Err(_) => return dir,
                Ok(m) if m.is_dir() && m.uid() == uid => return dir,
                Ok(_) => {}
            }
            return tmp.join(format!("rollcopy-{uid}-{}", std::process::id()));
        }
    }
    tmp.join(format!("rollcopy-{}", std::process::id()))
}
