//! Which app "Open" hands a file to, and whether it can likely play it.
//!
//! iPhone videos are HEVC (often 10-bit HDR) with AAC sound; videos saved
//! by other apps (Snapchat, WhatsApp) are usually H.264 + AAC. A fresh
//! Ubuntu has a video player, but its GStreamer decodes none of these until
//! `gstreamer1.0-libav` (or `ubuntu-restricted-extras`) is installed, so
//! the player opens and plays nothing while `xdg-open` reports success.
//! Found in an Ubuntu 26.04 VM, 2026-11-13, with both kinds of video.
//! Players that bring their own decoders (VLC, mpv, …) are fine.

use std::path::{Path, PathBuf};

/// MIME type `xdg-mime` uses for a file name, for the types Rollcopy opens.
pub(crate) fn mime_for(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mov" => "video/quicktime",
        "mp4" | "m4v" => "video/mp4",
        "heic" | "heif" => "image/heic",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        _ => return None,
    })
}

/// Players that decode video themselves, whatever GStreamer has.
pub(crate) fn has_own_decoders(desktop_id: &str) -> bool {
    let id = desktop_id.to_ascii_lowercase();
    [
        "vlc",
        "mpv",
        "celluloid",
        "haruna",
        "smplayer",
        "mplayer",
        "kodi",
    ]
    .iter()
    .any(|p| id.contains(p))
}

/// The desktop file id (`org.gnome.Showtime.desktop`) of the default app
/// for a MIME type, if one is set.
pub(crate) fn default_app(mime: &str) -> Option<String> {
    let out = std::process::Command::new("xdg-mime")
        .args(["query", "default", mime])
        .output()
        .ok()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!id.is_empty()).then_some(id)
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    {
        dirs.push(home);
    }
    let system =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".to_owned());
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs
}

/// The app's display name (`Videos`), from its desktop file.
pub(crate) fn app_name(desktop_id: &str) -> Option<String> {
    data_dirs().into_iter().find_map(|d| {
        let text = std::fs::read_to_string(d.join("applications").join(desktop_id)).ok()?;
        name_from_desktop_file(&text)
    })
}

/// `Name=` of the `[Desktop Entry]` group (the untranslated one).
fn name_from_desktop_file(text: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry && let Some(name) = line.strip_prefix("Name=") {
            return Some(name.trim().to_owned());
        }
    }
    None
}

/// Whether GStreamer has the libav plugin, which decodes H.264, HEVC and
/// AAC. Checked by plugin file, since `gst-inspect-1.0` isn't always
/// installed.
pub(crate) fn gstreamer_has_libav() -> bool {
    gstreamer_has_libav_in(&gst_plugin_dirs())
}

fn gst_plugin_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var("GST_PLUGIN_SYSTEM_PATH_1_0")
        .ok()
        .map(|p| p.split(':').map(PathBuf::from).collect())
        .unwrap_or_default();
    for lib in ["/usr/lib", "/usr/lib64", "/usr/local/lib"] {
        dirs.push(Path::new(lib).join("gstreamer-1.0"));
        if let Ok(entries) = std::fs::read_dir(lib) {
            for e in entries.flatten() {
                dirs.push(e.path().join("gstreamer-1.0"));
            }
        }
    }
    dirs
}

fn gstreamer_has_libav_in(dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|d| d.join("libgstlibav.so").exists())
}

/// What to tell the user after asking the system to open `name`: an
/// error if no app is set for it, or a hint if the app probably can't
/// play iPhone video.
pub(crate) enum Check {
    Fine { app: Option<String> },
    MayNotPlay { app: Option<String>, hint: String },
    NoApp { error: String },
}

pub(crate) fn check(name: &str) -> Check {
    let Some(mime) = mime_for(name) else {
        return Check::Fine { app: None };
    };
    let Some(id) = default_app(mime) else {
        let what = if mime.starts_with("video/") {
            "videos"
        } else {
            "photos"
        };
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        return Check::NoApp {
            error: format!(
                "no app is set to open .{ext} {what}. Install one, for example VLC: sudo apt install vlc"
            ),
        };
    };
    let app = app_name(&id);
    if mime.starts_with("video/") && !has_own_decoders(&id) && !gstreamer_has_libav() {
        return Check::MayNotPlay {
            app,
            hint: "this player is missing the video decoders iPhone and app videos need. \
                   Install them with sudo apt install gstreamer1.0-libav, or use VLC."
                .to_owned(),
        };
    }
    Check::Fine { app }
}

/// A plain message for a failed `xdg-open` (its exit codes: 2 file not
/// found, 3 a required tool missing, 4 the action failed, which is what
/// "no app for this kind of file" looks like).
pub(crate) fn xdg_open_error(name: &str, code: Option<i32>) -> String {
    let is_video = mime_for(name).is_some_and(|m| m.starts_with("video/"));
    let install = if is_video {
        "Install a video player, for example VLC: sudo apt install vlc"
    } else {
        "Install an image viewer that opens this kind of file"
    };
    match code {
        Some(3) => "xdg-open is missing a tool it needs (is xdg-utils installed?)".to_owned(),
        Some(4) => format!("no app could open {name}. {install}"),
        Some(c) => format!("the system couldn't open {name} (xdg-open exit code {c}). {install}"),
        None => format!("the system couldn't open {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_open_failures_read_plainly() {
        let msg = xdg_open_error("IMG_0001.MOV", Some(4));
        assert!(msg.starts_with("no app could open IMG_0001.MOV"), "{msg}");
        assert!(msg.contains("sudo apt install vlc"));
        assert!(xdg_open_error("IMG_0001.HEIC", Some(4)).contains("image viewer"));
    }

    #[test]
    fn mime_types() {
        assert_eq!(mime_for("IMG_0001.MOV"), Some("video/quicktime"));
        assert_eq!(mime_for("IMG_0001-edited.mp4"), Some("video/mp4"));
        assert_eq!(mime_for("IMG_0001.HEIC"), Some("image/heic"));
        assert_eq!(mime_for("README"), None);
    }

    #[test]
    fn players_with_their_own_decoders() {
        assert!(has_own_decoders("vlc.desktop"));
        assert!(has_own_decoders("org.videolan.VLC.desktop"));
        assert!(has_own_decoders(
            "io.github.celluloid_player.Celluloid.desktop"
        ));
        assert!(!has_own_decoders("org.gnome.Showtime.desktop"));
        assert!(!has_own_decoders("org.gnome.Totem.desktop"));
    }

    #[test]
    fn desktop_file_name() {
        let text = "[Desktop Entry]\nName[de]=Videos\nName=Videos\nExec=x\n[Desktop Action new]\nName=New\n";
        assert_eq!(name_from_desktop_file(text), Some("Videos".to_owned()));
        assert_eq!(name_from_desktop_file("[Other]\nName=X\n"), None);
    }

    #[test]
    fn libav_plugin_detection() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        assert!(!gstreamer_has_libav_in(&dirs));
        std::fs::write(dir.path().join("libgstlibav.so"), b"").unwrap();
        assert!(gstreamer_has_libav_in(&dirs));
    }
}
