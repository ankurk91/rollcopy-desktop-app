//! Saved preferences (PLAN §8a) and window state.
//!
//! - `~/.config/rollcopy/config.toml`: preferences. Every field has a default,
//!   so older files still load; unknown keys are ignored. Writes go to a temp
//!   file that is renamed into place. A file that doesn't parse is renamed to
//!   `config.toml.bad` and defaults are used, with a one-line notice.
//! - `~/.local/state/rollcopy/ui.toml`: window size and position, preview
//!   width, queue panel state, last update check. Same rules, no notice.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutPref {
    #[default]
    ByDate,
    Flat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictPref {
    #[default]
    Ask,
    Skip,
    Overwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VideoPref {
    /// Lossless MP4 remux (decided 2026-11-12).
    #[default]
    Lossless,
    H264,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Destination {
    /// Base folder; `~/Pictures/Rollcopy` by default.
    pub base_folder: PathBuf,
    /// Put each phone in `<base>/<device name>/`.
    pub subfolder_per_device: bool,
    pub layout: LayoutPref,
    pub on_conflict: ConflictPref,
    /// Folder picked with the toolbar's 📁, per phone (UDID). Overrides the
    /// base + device-name default.
    pub devices: BTreeMap<String, PathBuf>,
}

impl Default for Destination {
    fn default() -> Self {
        Self {
            base_folder: default_base_folder(),
            subfolder_per_device: true,
            layout: LayoutPref::ByDate,
            on_conflict: ConflictPref::Ask,
            devices: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Conversion {
    pub jpeg_quality: u8,
    pub video: VideoPref,
}

impl Default for Conversion {
    fn default() -> Self {
        Self {
            jpeg_quality: 92,
            video: VideoPref::Lossless,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPrefs {
    /// List Recently Deleted in the tree (greyed, never copied). Off by
    /// default: iOS locks that album too (PLAN §3).
    pub show_recently_deleted: bool,
    /// List the phone's Hidden album, under its own node (audit SEC-01).
    pub show_hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPrefs {
    pub theme: ThemePref,
    /// At most once a day (PLAN §8c); off by default.
    pub check_updates_on_startup: bool,
    /// Closing the window while copying keeps Rollcopy in the tray (PLAN §6).
    pub keep_in_tray: bool,
    /// Desktop notification when a batch of copies finishes.
    pub notify_when_done: bool,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            theme: ThemePref::System,
            check_updates_on_startup: false,
            keep_in_tray: true,
            notify_when_done: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub destination: Destination,
    pub conversion: Conversion,
    pub library: LibraryPrefs,
    pub app: AppPrefs,
}

impl Config {
    /// Where a phone's files go: the folder picked for it, else
    /// `<base>/<device name>` (or `<base>` without per-device subfolders).
    pub fn destination_for(&self, udid: &str, device_name: &str) -> PathBuf {
        if let Some(p) = self.destination.devices.get(udid) {
            return p.clone();
        }
        let base = &self.destination.base_folder;
        if self.destination.subfolder_per_device {
            base.join(safe_folder_name(device_name))
        } else {
            base.clone()
        }
    }

    pub fn copy_settings(&self, dest: PathBuf) -> rollcopy_queue::Settings {
        let mut s = rollcopy_queue::Settings::new(dest);
        s.layout = match self.destination.layout {
            LayoutPref::ByDate => rollcopy_queue::Layout::ByDate,
            LayoutPref::Flat => rollcopy_queue::Layout::Flat,
        };
        s.on_conflict = match self.destination.on_conflict {
            ConflictPref::Ask => rollcopy_queue::ConflictPolicy::Ask,
            ConflictPref::Skip => rollcopy_queue::ConflictPolicy::Skip,
            ConflictPref::Overwrite => rollcopy_queue::ConflictPolicy::Overwrite,
        };
        s.jpeg_quality = self.conversion.jpeg_quality.clamp(50, 100);
        s.video_mode = match self.conversion.video {
            VideoPref::Lossless => rollcopy_convert::VideoMode::Remux,
            VideoPref::H264 => rollcopy_convert::VideoMode::H264,
        };
        s
    }
}

pub fn default_base_folder() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.picture_dir().map(Path::to_path_buf))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Pictures")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Rollcopy")
}

/// A device name usable as a folder name.
pub fn safe_folder_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    if s.trim().is_empty() {
        "iPhone".to_owned()
    } else {
        s
    }
}

/// Window geometry in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SavedBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    pub window: Option<SavedBounds>,
    pub preview_width: f32,
    pub transfers_expanded: bool,
    /// Unix seconds of the last update check (for the once-a-day limit).
    pub last_update_check: Option<i64>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            window: None,
            preview_width: 300.,
            transfers_expanded: true,
            last_update_check: None,
        }
    }
}

/// Loads a TOML file. Missing file → defaults. Broken file → renamed to
/// `<name>.bad`, defaults, and a notice to show the user.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> (T, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (T::default(), None),
        Err(e) => {
            return (
                T::default(),
                Some(format!("Couldn't read {}: {e}", path.display())),
            );
        }
    };
    match toml::from_str(&text) {
        Ok(v) => (v, None),
        Err(e) => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = std::fs::rename(path, &bad);
            let first = e.to_string().lines().next().unwrap_or_default().to_owned();
            (
                T::default(),
                Some(format!(
                    "Settings file was broken ({first}); saved it as {} and started with defaults",
                    PathBuf::from(bad).display()
                )),
            )
        }
    }
}

/// Creates `dir` (and missing parents) readable by this user only, and
/// tightens it if it already exists. Rollcopy's cache and state hold
/// thumbnails, photo copies and the phone's library database.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        b.mode(0o700);
        b.create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    b.create(dir)
}

/// Saves as TOML via a temp file + rename, so a crash never leaves a
/// half-written file.
pub fn save<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(value).map_err(std::io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, text)?;
    std::fs::File::open(&tmp)?.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_dirs_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b");
        create_private_dir(&dir).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        // An existing, looser folder is tightened.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
    }

    #[test]
    fn round_trips_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollcopy/config.toml");
        let mut c = Config::default();
        c.conversion.jpeg_quality = 85;
        c.conversion.video = VideoPref::H264;
        c.app.theme = ThemePref::Dark;
        c.destination
            .devices
            .insert("UDID1".into(), "/mnt/photos".into());
        save(&path, &c).unwrap();
        let (back, notice): (Config, _) = load(&path);
        assert_eq!(back, c);
        assert!(notice.is_none());
        assert!(!dir.path().join("rollcopy/config.toml.tmp").exists());
    }

    #[test]
    fn old_file_with_missing_and_unknown_keys_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[conversion]\njpeg_quality = 80\nfuture_option = true\n[something_new]\nx = 1\n",
        )
        .unwrap();
        let (c, notice): (Config, _) = load(&path);
        assert!(notice.is_none());
        assert_eq!(c.conversion.jpeg_quality, 80);
        assert_eq!(c.conversion.video, VideoPref::Lossless);
        assert!(c.destination.subfolder_per_device);
    }

    #[test]
    fn broken_file_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml [").unwrap();
        let (c, notice): (Config, _) = load(&path);
        assert_eq!(c, Config::default());
        assert!(notice.unwrap().contains("config.toml.bad"));
        assert!(dir.path().join("config.toml.bad").exists());
        assert!(!path.exists());
    }

    #[test]
    fn destination_rules() {
        let mut c = Config::default();
        c.destination.base_folder = "/p/Rollcopy".into();
        assert_eq!(
            c.destination_for("U", "My iPhone"),
            PathBuf::from("/p/Rollcopy/My iPhone")
        );
        c.destination.subfolder_per_device = false;
        assert_eq!(c.destination_for("U", "x"), PathBuf::from("/p/Rollcopy"));
        c.destination
            .devices
            .insert("U".into(), "/elsewhere".into());
        assert_eq!(c.destination_for("U", "x"), PathBuf::from("/elsewhere"));
        assert_eq!(safe_folder_name("a/b"), "a_b");
        assert_eq!(safe_folder_name(" "), "iPhone");
    }

    #[test]
    fn values_are_written_in_kebab_case() {
        let mut c = Config::default();
        c.destination.layout = LayoutPref::ByDate;
        let text = toml::to_string_pretty(&c).unwrap();
        assert!(text.contains("layout = \"by-date\""), "{text}");
        assert!(text.contains("video = \"lossless\""));
    }
}
