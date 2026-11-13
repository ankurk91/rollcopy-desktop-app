//! Where a copied item goes (PLAN §7.3).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use rollcopy_convert::VideoMode;
use rollcopy_library::{Asset, MediaKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// `<dest>/<YYYY>/<MM>/images|videos/<name>`
    #[default]
    ByDate,
    /// `<dest>/<name>`
    Flat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    #[default]
    Ask,
    Skip,
    Overwrite,
}

/// Copy settings (subset of PLAN §8a that the pipeline needs).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Destination for this device, e.g. `~/Pictures/Rollcopy/My iPhone`.
    pub dest: PathBuf,
    pub layout: Layout,
    pub on_conflict: ConflictPolicy,
    pub jpeg_quality: u8,
    pub video_mode: VideoMode,
}

impl Settings {
    pub fn new(dest: PathBuf) -> Self {
        Self {
            dest,
            layout: Layout::ByDate,
            on_conflict: ConflictPolicy::Ask,
            jpeg_quality: rollcopy_convert::DEFAULT_JPEG_QUALITY,
            video_mode: VideoMode::Remux,
        }
    }
}

/// File name an asset is saved under: the original stem with the output
/// extension (`IMG_0057.HEIC` edited → `IMG_0057.JPG`), never `FullSizeRender`.
pub fn output_name(a: &Asset) -> String {
    format!("{}.{}", a.stem(), a.save_plan().to_ext)
}

/// Full destination path for an asset. Uses the capture date in the zone it
/// was taken in; items without a date go to `Undated/`.
pub fn dest_path(settings: &Settings, a: &Asset) -> PathBuf {
    dest_path_in(&settings.dest, settings.layout, a)
}

pub fn dest_path_in(root: &Path, layout: Layout, a: &Asset) -> PathBuf {
    let name = output_name(a);
    match layout {
        Layout::Flat => root.join(name),
        Layout::ByDate => {
            let kind = match a.kind {
                MediaKind::Photo => "images",
                MediaKind::Video => "videos",
            };
            let dated = match a.year_month() {
                Some((y, m)) => root.join(format!("{y:04}")).join(format!("{m:02}")),
                None => root.join("Undated"),
            };
            dated.join(kind).join(name)
        }
    }
}

/// The `n`-th name tried for a destination: `IMG_0057.JPG`, then
/// `IMG_0057 (2).JPG`, `IMG_0057 (3).JPG`, …
fn candidate(base: &Path, n: u32) -> PathBuf {
    if n <= 1 {
        return base.to_path_buf();
    }
    let stem = base.file_stem().unwrap_or_default().to_string_lossy();
    let name = match base.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    base.with_file_name(name)
}

/// Whether a file modified at `modified` is Rollcopy's copy of `a`.
///
/// Every copy gets the capture time as its modified time, so a file with
/// another time is a different photo with the same name (iOS reuses
/// `IMG_0001`–`IMG_9999` across `DCIM/1xxAPPLE` folders) or a file from
/// another tool (audit REL-01). Allowed slack: 2 s for FAT's resolution,
/// plus whole quarter hours within ±14 h, because FAT stores local time and
/// reads back shifted after a time-zone or DST change.
///
/// Undated items can't be told apart; any file there counts as theirs.
pub fn is_copy_of(modified: SystemTime, a: &Asset) -> bool {
    let Some(captured) = a.captured else {
        return true;
    };
    let captured = SystemTime::from(captured);
    let diff = match modified.duration_since(captured) {
        Ok(d) => d,
        Err(e) => e.duration(),
    };
    if diff > Duration::from_secs(14 * 3600 + 2) {
        return false;
    }
    let off = diff.as_secs_f64() % 900.0;
    off <= 2.0 || off >= 898.0
}

/// Where an asset's copy goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Nothing is there: write a new file.
    Free(PathBuf),
    /// This asset's earlier copy is there; the conflict policy decides.
    Copied(PathBuf),
}

impl Target {
    pub fn path(&self) -> &Path {
        match self {
            Target::Free(p) | Target::Copied(p) => p,
        }
    }
}

/// Picks the destination for `a`. It's the first name in [`dest_path`],
/// `… (2)`, `… (3)`, … that holds this asset's earlier copy, or else the
/// first free one. Files that aren't a copy of `a` are never chosen, so they
/// are never overwritten. `taken` marks paths other queued items will write.
pub fn resolve_dest(settings: &Settings, a: &Asset, taken: impl Fn(&Path) -> bool) -> Target {
    let base = dest_path(settings, a);
    for n in 1.. {
        let path = candidate(&base, n);
        if taken(&path) {
            continue;
        }
        match std::fs::metadata(&path) {
            Ok(m) => {
                if m.modified().is_ok_and(|t| is_copy_of(t, a)) {
                    return Target::Copied(path);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Target::Free(path),
            // Unreadable: leave it alone and try the next name.
            Err(_) => {}
        }
    }
    unreachable!("ran out of candidate names")
}

/// Where this asset's earlier copy is, if it was copied. Stops at the first
/// name with nothing there.
pub fn find_copy(settings: &Settings, a: &Asset) -> Option<PathBuf> {
    match resolve_dest(settings, a, |_| false) {
        Target::Copied(p) => Some(p),
        Target::Free(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn by_date_uses_capture_zone() {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_3480.MOV", MediaKind::Video);
        // 31 Aug 20:00 UTC = 1 Sep 01:30 in +05:30.
        a.captured = Some(Utc.with_ymd_and_hms(2025, 8, 31, 20, 0, 0).unwrap());
        a.tz_offset_secs = Some(19800);
        let s = Settings::new("/dest".into());
        assert_eq!(
            dest_path(&s, &a),
            PathBuf::from("/dest/2025/09/videos/IMG_3480.MP4")
        );
    }

    #[test]
    fn edited_and_flat_and_undated() {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_0057.HEIC", MediaKind::Photo);
        a.edited_render = Some(
            "/PhotoData/Mutations/DCIM/100APPLE/IMG_0057/Adjustments/FullSizeRender.jpg".into(),
        );
        let mut s = Settings::new("/dest".into());
        assert_eq!(
            dest_path(&s, &a),
            PathBuf::from("/dest/Undated/images/IMG_0057.JPG")
        );
        s.layout = Layout::Flat;
        assert_eq!(dest_path(&s, &a), PathBuf::from("/dest/IMG_0057.JPG"));
    }

    fn dated(dir: &str, name: &str, secs: i64) -> Asset {
        let mut a = Asset::new(dir, name, MediaKind::Photo);
        a.captured = Some(Utc.timestamp_opt(secs, 0).unwrap());
        a
    }

    fn touch(path: &Path, a: &Asset) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::from(a.captured.unwrap()))
            .unwrap();
    }

    #[test]
    fn candidates() {
        let base = Path::new("/d/IMG_0057.JPG");
        assert_eq!(candidate(base, 1), base);
        assert_eq!(candidate(base, 2), Path::new("/d/IMG_0057 (2).JPG"));
        assert_eq!(candidate(Path::new("/d/x"), 3), Path::new("/d/x (3)"));
    }

    #[test]
    fn copy_detection_tolerates_fat_times() {
        let a = dated("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
        let t = SystemTime::from(a.captured.unwrap());
        assert!(is_copy_of(t, &a));
        assert!(is_copy_of(t + Duration::from_secs(1), &a));
        assert!(is_copy_of(t - Duration::from_secs(2), &a));
        assert!(is_copy_of(t + Duration::from_secs(3600), &a));
        assert!(is_copy_of(t - Duration::from_secs(5 * 3600 + 1800 + 1), &a));
        assert!(!is_copy_of(t + Duration::from_secs(60), &a));
        assert!(!is_copy_of(t + Duration::from_secs(3 * 86400), &a));
        let undated = Asset::new("DCIM/100APPLE", "IMG_0058.JPG", MediaKind::Photo);
        assert!(is_copy_of(t, &undated));
    }

    /// Two photos named IMG_0057 from different DCIM folders, same month.
    #[test]
    fn same_name_photos_get_their_own_files() {
        let dir = tempfile::tempdir().unwrap();
        for layout in [Layout::ByDate, Layout::Flat] {
            let mut s = Settings::new(dir.path().join(format!("{layout:?}")));
            s.layout = layout;
            let first = dated("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
            let second = dated("DCIM/110APPLE", "IMG_0057.JPG", 1_700_500_000);
            let base = dest_path(&s, &first);
            assert_eq!(base, dest_path(&s, &second));

            // Nothing copied yet.
            assert_eq!(
                resolve_dest(&s, &first, |_| false),
                Target::Free(base.clone())
            );
            assert_eq!(find_copy(&s, &second), None);
            // Queued together: the second avoids the first's reservation.
            let t = resolve_dest(&s, &second, |p| p == base);
            assert_eq!(t, Target::Free(candidate(&base, 2)));

            // After only the first is copied, the second is still not copied
            // and goes next to it, never over it.
            touch(&base, &first);
            assert_eq!(find_copy(&s, &first), Some(base.clone()));
            assert_eq!(find_copy(&s, &second), None);
            assert_eq!(
                resolve_dest(&s, &second, |_| false),
                Target::Free(candidate(&base, 2))
            );
            touch(&candidate(&base, 2), &second);
            assert_eq!(find_copy(&s, &second), Some(candidate(&base, 2)));
            assert_eq!(
                resolve_dest(&s, &first, |_| false),
                Target::Copied(base.clone())
            );
        }
    }

    #[test]
    fn foreign_file_is_never_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings::new(dir.path().to_path_buf());
        let a = dated("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
        let base = dest_path(&s, &a);
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        std::fs::write(&base, b"someone else's").unwrap();
        assert_eq!(find_copy(&s, &a), None);
        assert_eq!(
            resolve_dest(&s, &a, |_| false),
            Target::Free(candidate(&base, 2))
        );
    }
}
