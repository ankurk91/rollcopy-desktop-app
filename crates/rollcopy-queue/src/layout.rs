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
pub(crate) fn candidate(base: &Path, n: u32) -> PathBuf {
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

/// Whether a file with this modified time and size is Rollcopy's copy of
/// `a`. With `precise_times` (see `space::precise_times`) the time must
/// match within 2 s; otherwise the FAT slack of [`is_copy_of`] applies.
/// That slack applied everywhere once, so with Overwrite on ext4 a
/// different photo of the same name taken a whole number of quarter hours
/// apart was taken for the copy and replaced (review C8). A byte-for-byte
/// copy must also have the original's size.
pub(crate) fn copy_matches(modified: SystemTime, len: u64, a: &Asset, precise_times: bool) -> bool {
    let time_ok = match a.captured {
        None => true,
        Some(c) if precise_times => {
            let c = SystemTime::from(c);
            let diff = modified.duration_since(c).unwrap_or_else(|e| e.duration());
            diff <= Duration::from_secs(2)
        }
        Some(_) => is_copy_of(modified, a),
    };
    if !time_ok {
        return false;
    }
    let plan = a.save_plan();
    match a.original_size {
        Some(size) if !plan.converts() && !plan.uses_edited => len == size,
        _ => true,
    }
}

/// [`copy_matches`] for a file on disk.
pub(crate) fn holds_copy(m: &std::fs::Metadata, a: &Asset, precise_times: bool) -> bool {
    m.modified()
        .is_ok_and(|t| copy_matches(t, m.len(), a, precise_times))
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

/// Most `… (n)` names tried for one item before giving up.
const MAX_CANDIDATES: u32 = 10_000;

/// Picks the destination for `a`. It's the first name in [`dest_path`],
/// `… (2)`, `… (3)`, … that holds this asset's earlier copy, or else the
/// first free one. Files that aren't a copy of `a` are never chosen, so they
/// are never overwritten. `taken` marks paths other queued items will write.
///
/// Fails when a name can't be checked for any reason but "not there": a
/// file where a folder should be, no permission, or a share that went
/// away fail the same way for every name, so trying the next would never
/// end. Also fails after [`MAX_CANDIDATES`] names.
pub fn resolve_dest(
    settings: &Settings,
    a: &Asset,
    taken: impl Fn(&Path) -> bool,
) -> std::io::Result<Target> {
    let base = dest_path(settings, a);
    let precise = crate::space::precise_times(&base);
    for n in 1..=MAX_CANDIDATES {
        let path = candidate(&base, n);
        if taken(&path) {
            continue;
        }
        match std::fs::metadata(&path) {
            Ok(m) => {
                if holds_copy(&m, a, precise) {
                    return Ok(Target::Copied(path));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Target::Free(path)),
            Err(e) => {
                let folder = path.parent().unwrap_or(&path);
                return Err(std::io::Error::new(
                    e.kind(),
                    format!("can't use the folder {}: {e}", folder.display()),
                ));
            }
        }
    }
    Err(std::io::Error::other(format!(
        "no free file name for {} after {MAX_CANDIDATES} tries",
        base.display()
    )))
}

/// Where this asset's earlier copy is, if it was copied. Stops at the first
/// name with nothing there; a folder that can't be read counts as not
/// copied.
pub fn find_copy(settings: &Settings, a: &Asset) -> Option<PathBuf> {
    match resolve_dest(settings, a, |_| false) {
        Ok(Target::Copied(p)) => Some(p),
        Ok(Target::Free(_)) | Err(_) => None,
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

    /// A file where the year folder should be makes every name fail the
    /// same way: give up at once instead of trying names forever.
    #[test]
    fn unusable_folder_fails_fast() {
        let dir = tempfile::tempdir().unwrap();
        let a = dated("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
        let s = Settings::new(dir.path().to_path_buf());
        let year = dest_path(&s, &a)
            .strip_prefix(dir.path())
            .unwrap()
            .components()
            .next()
            .unwrap()
            .as_os_str()
            .to_owned();
        std::fs::write(dir.path().join(&year), b"not a folder").unwrap();
        let started = std::time::Instant::now();
        let err = resolve_dest(&s, &a, |_| false).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(err.to_string().contains("can't use the folder"), "{err}");
        assert_eq!(find_copy(&s, &a), None);
    }

    /// Every name taken by other queued items: an error, not a hang.
    #[test]
    fn gives_up_after_the_last_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let a = dated("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
        let s = Settings::new(dir.path().to_path_buf());
        let err = resolve_dest(&s, &a, |_| true).unwrap_err();
        assert!(err.to_string().contains("no free file name"), "{err}");
    }

    /// On ext4 and the like only an exact time (±2 s) counts; elsewhere
    /// the FAT slack stays. Straight copies must also match in size.
    #[test]
    fn exact_times_where_the_disk_keeps_them() {
        let mut a = dated("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
        a.original_size = Some(1000);
        let t = SystemTime::from(a.captured.unwrap());
        let hour = Duration::from_secs(3600);
        assert!(copy_matches(t, 1000, &a, true));
        assert!(copy_matches(t + Duration::from_secs(2), 1000, &a, true));
        assert!(!copy_matches(t + hour, 1000, &a, true));
        assert!(copy_matches(t + hour, 1000, &a, false));
        // Same time, different size: another file.
        assert!(!copy_matches(t, 999, &a, true));
        // Converted (HEIC → JPG): sizes differ by nature; time decides.
        let mut heic = dated("DCIM/100APPLE", "IMG_0058.HEIC", 1_700_000_000);
        heic.original_size = Some(1000);
        assert!(copy_matches(t, 640, &heic, true));
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
                resolve_dest(&s, &first, |_| false).unwrap(),
                Target::Free(base.clone())
            );
            assert_eq!(find_copy(&s, &second), None);
            // Queued together: the second avoids the first's reservation.
            let t = resolve_dest(&s, &second, |p| p == base).unwrap();
            assert_eq!(t, Target::Free(candidate(&base, 2)));

            // After only the first is copied, the second is still not copied
            // and goes next to it, never over it.
            touch(&base, &first);
            assert_eq!(find_copy(&s, &first), Some(base.clone()));
            assert_eq!(find_copy(&s, &second), None);
            assert_eq!(
                resolve_dest(&s, &second, |_| false).unwrap(),
                Target::Free(candidate(&base, 2))
            );
            touch(&candidate(&base, 2), &second);
            assert_eq!(find_copy(&s, &second), Some(candidate(&base, 2)));
            assert_eq!(
                resolve_dest(&s, &first, |_| false).unwrap(),
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
            resolve_dest(&s, &a, |_| false).unwrap(),
            Target::Free(candidate(&base, 2))
        );
    }
}
