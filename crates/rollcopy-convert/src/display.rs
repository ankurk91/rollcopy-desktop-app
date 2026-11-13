//! Screen-sized copies of photos for the full-window viewer.
//!
//! gpui draws image pixels as stored and ignores the EXIF orientation, so an
//! iPhone JPG shot in portrait would show sideways. The copy made here has
//! the rotation applied, is scaled to fit `max_side`, and keeps the colour
//! profile. These copies are only for viewing; copies saved to the
//! destination are made in `heic.rs` / byte for byte.

use std::path::Path;

use image::{DynamicImage, ImageDecoder, ImageReader, imageops::FilterType};

use crate::ConvertError;
use crate::heic::{decode_heic, write_jpeg};

const QUALITY: u8 = 88;

/// Writes a heavily blurred, tiny copy of an image (a thumbnail) to `dst`,
/// for Hidden-album items shown before the user chooses to see them. Only
/// colours survive: at 48 px wide and blurred, nothing is recognisable.
pub fn veil_jpeg(src: &Path, dst: &Path) -> Result<(), ConvertError> {
    let img = image::open(src).map_err(|e| ConvertError::Decode(e.to_string()))?;
    let small = img.resize(48, 48, FilterType::Triangle).blur(4.0).to_rgb8();
    let size = small.dimensions();
    write_jpeg(dst, small.as_raw(), size, 70, None, None)
}

/// Pixel size of an image file, read from its header only.
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}

/// Writes a viewer JPEG of the photo at `src` to `dst`. HEIC/HEIF, JPEG and
/// PNG are read; other formats return `Decode`, and the caller can show the
/// file as it is.
pub fn display_jpeg(src: &Path, dst: &Path, max_side: u32) -> Result<(u32, u32), ConvertError> {
    let ext = src
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let (img, icc) = if matches!(ext.as_str(), "heic" | "heif") {
        let d = decode_heic(src)?;
        let img = image::RgbImage::from_raw(d.width, d.height, d.rgb)
            .ok_or_else(|| ConvertError::Decode("bad pixel buffer".into()))?;
        (DynamicImage::ImageRgb8(img), d.icc)
    } else {
        let mut decoder = ImageReader::open(src)?
            .with_guessed_format()?
            .into_decoder()
            .map_err(|e| ConvertError::Decode(e.to_string()))?;
        let orientation = decoder
            .orientation()
            .map_err(|e| ConvertError::Decode(e.to_string()))?;
        let icc = decoder.icc_profile().ok().flatten();
        let mut img =
            DynamicImage::from_decoder(decoder).map_err(|e| ConvertError::Decode(e.to_string()))?;
        img.apply_orientation(orientation);
        (img, icc)
    };
    let img = if img.width().max(img.height()) > max_side {
        img.resize(max_side, max_side, FilterType::Triangle)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let size = rgb.dimensions();
    write_jpeg(dst, rgb.as_raw(), size, QUALITY, icc, None)?;
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder as _;
    use image::codecs::jpeg::JpegEncoder;

    /// Big-endian EXIF (TIFF) with Orientation = 6 ("rotate 90° clockwise").
    fn exif_rotate_90() -> Vec<u8> {
        let mut t = b"MM\0\x2a\0\0\0\x08".to_vec();
        t.extend_from_slice(&[0, 1]); // one entry
        t.extend_from_slice(&[0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        t
    }

    #[test]
    fn applies_orientation_and_fits_the_screen() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("IMG_0001.JPG");
        // Stored 400×200 (landscape) with "rotate 90° clockwise".
        let mut f = std::fs::File::create(&src).unwrap();
        let mut enc = JpegEncoder::new_with_quality(&mut f, 90);
        enc.set_exif_metadata(exif_rotate_90()).unwrap();
        let px = vec![128u8; 400 * 200 * 3];
        image::ImageEncoder::write_image(enc, &px, 400, 200, image::ExtendedColorType::Rgb8)
            .unwrap();
        drop(f);

        let dst = dir.path().join("view.jpg");
        // Shown upright (portrait), scaled to fit 100 px.
        assert_eq!(display_jpeg(&src, &dst, 100).unwrap(), (50, 100));
        let back = image::open(&dst).unwrap();
        assert_eq!((back.width(), back.height()), (50, 100));
        // Small photos aren't enlarged.
        assert_eq!(display_jpeg(&src, &dst, 4000).unwrap(), (200, 400));
    }

    #[test]
    fn veil_is_tiny() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("5005.JPG");
        image::RgbImage::from_fn(480, 360, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 0])
        })
        .save(&src)
        .unwrap();
        let dst = dir.path().join("veil.jpg");
        veil_jpeg(&src, &dst).unwrap();
        let back = image::open(&dst).unwrap();
        assert_eq!((back.width(), back.height()), (48, 36));
    }

    #[test]
    fn unknown_formats_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("x.gif");
        std::fs::write(&src, b"GIF89a....").unwrap();
        assert!(display_jpeg(&src, &dir.path().join("o.jpg"), 100).is_err());
    }
}
