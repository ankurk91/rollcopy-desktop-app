//! Free disk space (audit REL-06).

use std::path::Path;

/// Bytes this user can still write on the disk holding `path`. `path` need
/// not exist yet; its nearest existing parent is asked. `None` when it
/// can't be told.
pub fn available(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    #[cfg(unix)]
    {
        let st = rustix::fs::statvfs(existing).ok()?;
        Some(st.f_bavail.saturating_mul(st.f_frsize))
    }
    #[cfg(not(unix))]
    {
        let _ = existing;
        None
    }
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else {
        format!("{:.0} MB", (b / 1e6).max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn asks_the_nearest_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("2026/11/images/IMG_0001.JPG");
        let free = available(&deep).unwrap();
        assert!(free > 0);
    }

    #[test]
    fn sizes() {
        assert_eq!(human(64 * 1024 * 1024), "67 MB");
        assert_eq!(human(2_500_000_000), "2.5 GB");
    }
}
