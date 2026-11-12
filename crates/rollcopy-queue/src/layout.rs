//! Where a copied item goes (PLAN §7.3).

use std::path::{Path, PathBuf};

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
}
