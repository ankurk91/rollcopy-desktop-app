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

/// Whether file times on the disk holding `path` read back exactly as set:
/// true for the usual Linux filesystems. FAT and exFAT store local time
/// (shifted after a time-zone or DST change), and FUSE and network shares
/// may sit on anything, so they count as not exact (review C8).
pub fn precise_times(path: &Path) -> bool {
    let Some(existing) = path.ancestors().find(|p| p.exists()) else {
        return false;
    };
    #[cfg(target_os = "linux")]
    {
        // ext2/3/4, btrfs, xfs, tmpfs, zfs, f2fs, bcachefs.
        const PRECISE: [u64; 7] = [
            0xEF53,
            0x9123_683E,
            0x5846_5342,
            0x0102_1994,
            0x2FC1_2FC1,
            0xF2F5_2010,
            0xCA45_1A4E,
        ];
        rustix::fs::statfs(existing)
            .is_ok_and(|st| PRECISE.contains(&(st.f_type as u64 & 0xFFFF_FFFF)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = existing;
        false
    }
}

/// The disk (device id) holding `path`, or its nearest existing parent.
pub fn device(path: &Path) -> Option<u64> {
    let existing = path.ancestors().find(|p| p.exists())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        std::fs::metadata(existing).ok().map(|m| m.dev())
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

    #[cfg(target_os = "linux")]
    #[test]
    fn tmpfs_and_ext4_times_are_exact() {
        // /dev/shm is tmpfs on every Linux this runs on.
        assert!(precise_times(Path::new("/dev/shm/rollcopy-test/x")));
        // procfs isn't in the list.
        assert!(!precise_times(Path::new("/proc/self")));
    }

    #[test]
    fn sizes() {
        assert_eq!(human(64 * 1024 * 1024), "67 MB");
        assert_eq!(human(2_500_000_000), "2.5 GB");
    }
}
