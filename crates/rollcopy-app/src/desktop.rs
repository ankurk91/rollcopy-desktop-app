//! Handing things to the desktop.

use std::path::Path;

/// Opens the file manager at `file`'s folder, with the file selected where
/// that works well (`org.freedesktop.FileManager1.ShowItems`: Nautilus,
/// Dolphin…). Nemo (Linux Mint) answers ShowItems by starting `nemo <file>`,
/// which took a long time to show a window, so with Nemo the folder is
/// simply opened. Doesn't block.
pub fn show_in_folder(file: &Path) {
    let file = file.to_path_buf();
    std::thread::spawn(move || {
        if crate::flatpak::active() {
            // The sandbox can't reach the file manager directly; the
            // portal opens the folder with the file selected.
            if let Err(e) = portal_open_directory(&file) {
                crate::applog!("show in folder: portal failed ({e}); opening the folder");
                if let Some(dir) = file.parent() {
                    let _ = std::process::Command::new("xdg-open").arg(dir).status();
                }
            }
            return;
        }
        let selected = !file_manager_is_nemo() && show_items(&file);
        if !selected && let Some(dir) = file.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).status();
        }
    });
}

/// Asks the desktop portal to show `file` in its folder
/// (`org.freedesktop.portal.OpenURI.OpenDirectory`). The file is passed as
/// an open file descriptor, so the portal sees the host's real file, not
/// the sandbox's view of it.
fn portal_open_directory(file: &Path) -> Result<(), String> {
    use std::collections::HashMap;
    use zbus::zvariant::{Fd, Value};
    let f = std::fs::File::open(file).map_err(|e| e.to_string())?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| e.to_string())?;
        let options: HashMap<&str, Value<'_>> = HashMap::new();
        conn.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.OpenURI"),
            "OpenDirectory",
            &("", Fd::from(&f), options),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
    })
}

/// Whether the FileManager1 service belongs to Nemo (or nemo-desktop).
fn file_manager_is_nemo() -> bool {
    let out = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.GetConnectionUnixProcessID",
            "string:org.freedesktop.FileManager1",
        ])
        .output();
    let Ok(out) = out else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let pid = text
        .split_whitespace()
        .skip_while(|w| *w != "uint32")
        .nth(1)
        .and_then(|p| p.parse::<u32>().ok());
    pid.and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok())
        .is_some_and(|comm| comm.trim().starts_with("nemo"))
}

/// Opens a folder in the file manager. Doesn't block.
pub fn open_folder(dir: &Path) {
    let dir = dir.to_path_buf();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open").arg(dir).status();
    });
}

fn show_items(file: &Path) -> bool {
    let uri = file_uri(file);
    std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.FileManager1",
            "--type=method_call",
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1.ShowItems",
        ])
        .arg(format!("array:string:{uri}"))
        .arg("string:")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `file://` URI for an absolute path, percent-encoding what URIs don't
/// allow (spaces, `#`, `%`, non-ASCII…). Commas would end a dbus-send
/// array item, so they're encoded too.
pub(crate) fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::file_uri;
    use std::path::Path;

    #[test]
    fn uris_are_encoded() {
        assert_eq!(
            file_uri(Path::new(
                "/home/me/Pictures/My iPhone/2026/11/IMG_0001 (2).JPG"
            )),
            "file:///home/me/Pictures/My%20iPhone/2026/11/IMG_0001%20%282%29.JPG"
        );
        assert_eq!(file_uri(Path::new("/a,b/é")), "file:///a%2Cb/%C3%A9");
    }
}
