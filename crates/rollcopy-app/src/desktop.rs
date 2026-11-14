//! Handing things to the desktop.

use std::path::Path;

/// Opens the file manager at `file`'s folder with the file selected
/// (`org.freedesktop.FileManager1.ShowItems`: Nemo, Nautilus, Dolphin…),
/// or just opens the folder when that isn't available. Doesn't block.
pub fn show_in_folder(file: &Path) {
    let file = file.to_path_buf();
    std::thread::spawn(move || {
        if !show_items(&file)
            && let Some(dir) = file.parent()
        {
            let _ = std::process::Command::new("xdg-open").arg(dir).status();
        }
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
