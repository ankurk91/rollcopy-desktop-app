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
    /// A screenshot (or screen recording) taken on the phone.
    pub screenshot: bool,
    /// (latitude, longitude), if recorded.
    pub location: Option<(f64, f64)>,
    pub original_size: Option<u64>,
    /// Device path of the edited version (`.../Adjustments/FullSizeRender.*`),
    /// if the item was edited on the phone.
    pub edited_render: Option<String>,
    /// When the item was last edited on the phone (Unix seconds), from the
    /// Photos DB. Changes on every edit, so caches can key on it.
    pub edited_at: Option<i64>,
    /// Where the item came from, from the app that saved it: "Camera",
    /// "Snapchat", "WhatsApp", "Screenshots", … (Photos DB only).
    pub source_app: Option<String>,
    /// Camera make and model from EXIF, e.g. "Apple iPhone 15".
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// Shot settings, e.g. "ISO 160 · f/1.6 · 1/33 s · 5.96 mm".
    pub exposure: Option<String>,
    /// Street address the phone looked up for the location.
    pub place: Option<String>,
    /// User albums the item is in.
    pub albums: Vec<String>,
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
            screenshot: false,
            location: None,
            original_size: None,
            edited_render: None,
            edited_at: None,
            source_app: None,
            camera: None,
            lens: None,
            exposure: None,
            place: None,
            albums: Vec::new(),
        }
    }

    /// Absolute path of the original on the device's media filesystem.
    pub fn device_path(&self) -> String {
        format!("/{}/{}", self.directory, self.filename)
    }

    /// Whether the folder and file name are plain relative names. They end
    /// up in local paths (destination, thumbnail cache), so a damaged or
    /// hostile database must not be able to use `..`, absolute paths or
    /// stray separators to write elsewhere.
    pub fn has_safe_names(&self) -> bool {
        let plain = |c: &str| !c.is_empty() && c != "." && c != ".." && !c.contains(['\0', '\\']);
        plain(&self.filename)
            && !self.filename.contains('/')
            && self.directory.split('/').all(plain)
    }

    /// File name without extension (`IMG_0057`).
    pub fn stem(&self) -> &str {
        self.filename
            .rsplit_once('.')
            .map_or(self.filename.as_str(), |(s, _)| s)
    }

    /// A Live Photo's motion part as an item of its own: the `.MOV` with the
    /// same name next to the photo in `DCIM` (where iOS keeps it). Same
    /// capture time, so it's sorted and recognised as copied like the photo.
    /// `None` for anything that isn't a Live Photo.
    pub fn live_motion(&self) -> Option<Asset> {
        if !self.live_photo || self.kind != MediaKind::Photo {
            return None;
        }
        let mut m = Asset::new(
            self.directory.clone(),
            format!("{}.MOV", self.stem()),
            MediaKind::Video,
        );
        m.captured = self.captured;
        m.tz_offset_secs = self.tz_offset_secs;
        m.location = self.location;
        m.favorite = self.favorite;
        m.source_app = self.source_app.clone();
        m.camera = self.camera.clone();
        m.place = self.place.clone();
        m.albums = self.albums.clone();
        Some(m)
    }

    /// Taken with the front camera (from the lens name, Photos DB only).
    pub fn is_selfie(&self) -> bool {
        self.lens
            .as_deref()
            .is_some_and(|l| l.to_ascii_lowercase().contains("front"))
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
            .unwrap_or(chrono::Offset::fix(&Utc));
        Some(utc.with_timezone(&offset))
    }

    /// (year, month) of the local capture date, for the library tree.
    pub fn year_month(&self) -> Option<(i32, u32)> {
        self.captured_local().map(|d| (d.year(), d.month()))
    }
}

/// What a copy of this asset produces (PLAN §7.1–7.2): the edited version
/// if there is one, HEIC → JPG, MOV → MP4, everything else unchanged. Live
/// Photos are copied as the still only, so they follow the photo rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePlan {
    /// Device path that will be read.
    pub source: String,
    /// Upper-case extension of `source` (`HEIC`).
    pub from_ext: String,
    /// Upper-case extension written (`JPG`).
    pub to_ext: String,
    pub uses_edited: bool,
}

impl SavePlan {
    pub fn converts(&self) -> bool {
        self.from_ext != self.to_ext
    }
}

impl Asset {
    pub fn save_plan(&self) -> SavePlan {
        let source = self
            .edited_render
            .clone()
            .unwrap_or_else(|| self.device_path());
        let from_ext = source
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_uppercase())
            .unwrap_or_default();
        let to_ext = match from_ext.as_str() {
            "HEIC" | "HEIF" => "JPG".to_owned(),
            "MOV" => "MP4".to_owned(),
            "JPEG" => "JPG".to_owned(),
            other => other.to_owned(),
        };
        SavePlan {
            uses_edited: self.edited_render.is_some(),
            source,
            from_ext,
            to_ext,
        }
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
    fn save_plans() {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_0057.HEIC", MediaKind::Photo);
        let p = a.save_plan();
        assert_eq!((p.from_ext.as_str(), p.to_ext.as_str()), ("HEIC", "JPG"));
        assert!(p.converts() && !p.uses_edited);

        a.edited_render = Some(
            "/PhotoData/Mutations/DCIM/100APPLE/IMG_0057/Adjustments/FullSizeRender.jpg".into(),
        );
        let p = a.save_plan();
        assert_eq!(p.source, a.edited_render.clone().unwrap());
        assert_eq!((p.from_ext.as_str(), p.to_ext.as_str()), ("JPG", "JPG"));
        assert!(p.uses_edited);

        let v = Asset::new("DCIM/100APPLE", "IMG_0042.MOV", MediaKind::Video);
        assert_eq!(v.save_plan().to_ext, "MP4");
        let png = Asset::new("DCIM/100APPLE", "IMG_0001.PNG", MediaKind::Photo);
        assert!(!png.save_plan().converts());
    }

    #[test]
    fn stem_and_path() {
        let a = Asset::new("DCIM/100APPLE", "IMG_0057.HEIC", MediaKind::Photo);
        assert_eq!(a.stem(), "IMG_0057");
        assert_eq!(a.device_path(), "/DCIM/100APPLE/IMG_0057.HEIC");
    }

    #[test]
    fn live_photo_motion_is_the_mov_beside_it() {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_0001.HEIC", MediaKind::Photo);
        assert!(a.live_motion().is_none());
        a.live_photo = true;
        let m = a.live_motion().unwrap();
        assert_eq!(m.device_path(), "/DCIM/100APPLE/IMG_0001.MOV");
        assert_eq!(m.kind, MediaKind::Video);
        assert!(!m.live_photo && m.live_motion().is_none());
        assert_eq!(m.save_plan().to_ext, "MP4");
    }

    #[test]
    fn selfies_come_from_the_front_lens() {
        let mut a = Asset::new("DCIM/100APPLE", "IMG_0001.HEIC", MediaKind::Photo);
        assert!(!a.is_selfie());
        a.lens = Some("iPhone front camera 2.69mm f/1.9".into());
        assert!(a.is_selfie());
        a.lens = Some("iPhone back dual wide camera 6.86mm f/1.78".into());
        assert!(!a.is_selfie());
    }

    #[test]
    fn only_plain_names_are_safe() {
        let ok = Asset::new("DCIM/100APPLE", "IMG_0001.HEIC", MediaKind::Photo);
        assert!(ok.has_safe_names());
        for (dir, file) in [
            ("DCIM/100APPLE", "../../.bashrc"),
            ("DCIM/../../..", "IMG_0001.HEIC"),
            ("/etc", "IMG_0001.HEIC"),
            ("DCIM//100APPLE", "IMG_0001.HEIC"),
            ("DCIM/100APPLE", ".."),
            ("DCIM/100APPLE", "a/b.JPG"),
            ("DCIM/100APPLE", "a\\b.JPG"),
            ("DCIM/100APPLE", ""),
        ] {
            assert!(
                !Asset::new(dir, file, MediaKind::Photo).has_safe_names(),
                "{dir} {file}"
            );
        }
    }
}
