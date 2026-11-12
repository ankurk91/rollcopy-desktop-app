use chrono::{DateTime, Datelike, FixedOffset, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Photo,
    Video,
}

/// One item in the library: a photo or a video as the user sees it in Photos.
/// A Live Photo is one asset (the still); its motion clip is not an asset.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    /// Folder relative to the media root, e.g. `DCIM/100APPLE`.
    pub directory: String,
    /// File name in that folder, e.g. `IMG_0057.HEIC`.
    pub filename: String,
    pub kind: MediaKind,
    /// Capture time (UTC). From the Photos DB, or the file's mtime in the
    /// `DCIM/` fallback.
    pub captured: Option<DateTime<Utc>>,
    /// Offset of the time zone the item was captured in, in seconds east of UTC.
    pub tz_offset_secs: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Video length in seconds.
    pub duration_secs: Option<f64>,
    pub favorite: bool,
    pub hidden: bool,
    /// In Recently Deleted. Shown greyed out, never copied (PLAN §6).
    pub trashed: bool,
    pub live_photo: bool,
    /// (latitude, longitude), if recorded.
    pub location: Option<(f64, f64)>,
    pub original_size: Option<u64>,
    /// Device path of the edited version (`.../Adjustments/FullSizeRender.*`),
    /// if the item was edited on the phone.
    pub edited_render: Option<String>,
}

impl Asset {
    pub fn new(directory: impl Into<String>, filename: impl Into<String>, kind: MediaKind) -> Self {
        Self {
            directory: directory.into(),
            filename: filename.into(),
            kind,
            captured: None,
            tz_offset_secs: None,
            width: None,
            height: None,
            duration_secs: None,
            favorite: false,
            hidden: false,
            trashed: false,
            live_photo: false,
            location: None,
            original_size: None,
            edited_render: None,
        }
    }

    /// Absolute path of the original on the device's media filesystem.
    pub fn device_path(&self) -> String {
        format!("/{}/{}", self.directory, self.filename)
    }

    /// File name without extension (`IMG_0057`).
    pub fn stem(&self) -> &str {
        self.filename
            .rsplit_once('.')
            .map_or(self.filename.as_str(), |(s, _)| s)
    }

    pub fn is_edited(&self) -> bool {
        self.edited_render.is_some()
    }

    /// Capture time in the time zone it was taken in. Falls back to UTC when
    /// the zone is unknown. This is what year/month grouping and the
    /// destination layout use (PLAN §7.3).
    pub fn captured_local(&self) -> Option<DateTime<FixedOffset>> {
        let utc = self.captured?;
        let offset = self
            .tz_offset_secs
            .and_then(FixedOffset::east_opt)
            .unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset"));
        Some(utc.with_timezone(&offset))
    }

    /// (year, month) of the local capture date, for the library tree.
    pub fn year_month(&self) -> Option<(i32, u32)> {
        self.captured_local().map(|d| (d.year(), d.month()))
    }
}

/// Classifies a file by extension. `None` for files that are not media items
/// on their own (`.AAE` edit sidecars, unknown types).
pub fn kind_from_extension(filename: &str) -> Option<MediaKind> {
    let ext = filename.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "heic" | "heif" | "jpg" | "jpeg" | "png" | "gif" | "dng" | "tif" | "tiff" | "webp" => {
            Some(MediaKind::Photo)
        }
        "mov" | "mp4" | "m4v" | "hevc" => Some(MediaKind::Video),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn local_month_uses_capture_zone() {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_0001.HEIC", MediaKind::Photo);
        // 2025-08-31 20:00 UTC is already 1 Sep at +05:30.
        a.captured = Some(Utc.with_ymd_and_hms(2025, 8, 31, 20, 0, 0).unwrap());
        assert_eq!(a.year_month(), Some((2025, 8)));
        a.tz_offset_secs = Some(5 * 3600 + 1800);
        assert_eq!(a.year_month(), Some((2025, 9)));
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_from_extension("IMG_1.HEIC"), Some(MediaKind::Photo));
        assert_eq!(kind_from_extension("IMG_1.MOV"), Some(MediaKind::Video));
        assert_eq!(kind_from_extension("IMG_1.AAE"), None);
        assert_eq!(kind_from_extension("noext"), None);
    }

    #[test]
    fn stem_and_path() {
        let a = Asset::new("DCIM/100APPLE", "IMG_0057.HEIC", MediaKind::Photo);
        assert_eq!(a.stem(), "IMG_0057");
        assert_eq!(a.device_path(), "/DCIM/100APPLE/IMG_0057.HEIC");
    }
}
