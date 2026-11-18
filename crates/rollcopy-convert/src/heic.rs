//! HEIC → JPG (PLAN §7.2): decode with libheif (which applies the rotation
//! and mirroring stored in the file), encode as JPEG, keep EXIF with the
//! orientation reset, and embed the colour profile so Display P3 photos
//! don't look washed out.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, ImageEncoder};
use libheif_rs::{ColorSpace, HeifContext, ItemId, LibHeif, RgbChroma};

use crate::ConvertError;
use crate::exif;

/// Default JPEG quality (PLAN §8a).
pub const DEFAULT_JPEG_QUALITY: u8 = 92;

/// What a conversion produced, for logs and verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub has_exif: bool,
    pub has_icc: bool,
}

fn heif_err(e: libheif_rs::HeifError) -> ConvertError {
    ConvertError::Decode(e.to_string())
}

/// A decoded HEIC: rotated RGB pixels plus the metadata worth keeping.
pub(crate) struct Decoded {
    pub width: u32,
    pub height: u32,
    /// libheif's decoded image; [`Decoded::rgb`] lends its pixels.
    image: libheif_rs::Image,
    pub icc: Option<Vec<u8>>,
    /// EXIF as raw TIFF, orientation reset to 1.
    pub exif: Option<Vec<u8>>,
}

impl Decoded {
    /// The pixels as packed RGB rows. Borrowed from libheif when its rows
    /// are already packed (the usual case), so a 48 MP photo isn't copied
    /// (~146 MB) just to be encoded (review P3; audit PERF-06).
    pub(crate) fn rgb(&self) -> Result<std::borrow::Cow<'_, [u8]>, ConvertError> {
        let plane = self
            .image
            .planes()
            .interleaved
            .ok_or_else(|| ConvertError::Decode("no interleaved RGB plane".into()))?;
        let row = self.width as usize * 3;
        let h = self.height as usize;
        if plane.stride == row {
            let data = plane
                .data
                .get(..row * h)
                .ok_or_else(|| ConvertError::Decode("short pixel data".into()))?;
            return Ok(std::borrow::Cow::Borrowed(data));
        }
        let mut rgb = Vec::with_capacity(row * h);
        for y in 0..h {
            let start = y * plane.stride;
            rgb.extend_from_slice(
                plane
                    .data
                    .get(start..start + row)
                    .ok_or_else(|| ConvertError::Decode("short pixel row".into()))?,
            );
        }
        Ok(std::borrow::Cow::Owned(rgb))
    }
}

/// Decodes the primary image of a HEIC/HEIF file. libheif applies the
/// stored rotation and mirroring.
pub(crate) fn decode_heic(src: &Path) -> Result<Decoded, ConvertError> {
    let src_str = src
        .to_str()
        .ok_or_else(|| ConvertError::Decode("source path is not UTF-8".into()))?;
    let ctx = HeifContext::read_from_file(src_str).map_err(heif_err)?;
    let handle = ctx.primary_image_handle().map_err(heif_err)?;

    let lib = LibHeif::new();
    let image = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(heif_err)?;
    let (width, height) = {
        let plane = image
            .planes()
            .interleaved
            .ok_or_else(|| ConvertError::Decode("no interleaved RGB plane".into()))?;
        (plane.width, plane.height)
    };
    Ok(Decoded {
        width,
        height,
        image,
        icc: handle.color_profile_raw().map(|p| p.data),
        exif: read_exif(&handle),
    })
}

/// Writes RGB pixels as a JPEG at `dst` (overwritten), with an optional
/// colour profile and EXIF, and syncs it to disk.
pub(crate) fn write_jpeg(
    dst: &Path,
    rgb: &[u8],
    (w, h): (u32, u32),
    quality: u8,
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
) -> Result<(), ConvertError> {
    let mut out = BufWriter::new(File::create(dst)?);
    let mut enc = JpegEncoder::new_with_quality(&mut out, quality);
    if let Some(icc) = icc {
        enc.set_icc_profile(icc)
            .map_err(|e| ConvertError::Encode(e.to_string()))?;
    }
    if let Some(exif) = exif {
        enc.set_exif_metadata(exif)
            .map_err(|e| ConvertError::Encode(e.to_string()))?;
    }
    enc.write_image(rgb, w, h, ExtendedColorType::Rgb8)
        .map_err(|e| ConvertError::Encode(e.to_string()))?;
    out.flush()?;
    out.into_inner()
        .map_err(|e| ConvertError::Io(e.into_error()))?
        .sync_all()?;
    Ok(())
}

/// Converts the primary image of a HEIC/HEIF file to a JPEG at `dst`
/// (overwritten). Blocking and CPU-heavy: run it on a worker thread.
pub fn heic_to_jpeg(src: &Path, dst: &Path, quality: u8) -> Result<JpegInfo, ConvertError> {
    heic_to_jpeg_with(src, dst, quality, &crate::Cancel::new())
}

/// [`heic_to_jpeg`] that stops when `cancel` is set: nothing is written
/// after it, and an output written meanwhile is removed, so a cancelled job
/// can't leave a `.part` file behind (audit NEW-03).
pub fn heic_to_jpeg_with(
    src: &Path,
    dst: &Path,
    quality: u8,
    cancel: &crate::Cancel,
) -> Result<JpegInfo, ConvertError> {
    cancel.check()?;
    let d = decode_heic(src)?;
    cancel.check()?;
    let info = JpegInfo {
        width: d.width,
        height: d.height,
        has_exif: d.exif.is_some(),
        has_icc: d.icc.is_some(),
    };
    let written = d.rgb().and_then(|rgb| {
        write_jpeg(
            dst,
            &rgb,
            (d.width, d.height),
            quality,
            d.icc.clone(),
            d.exif.clone(),
        )
    });
    if let Err(e) = written {
        // Don't leave a half-written file; the caller's guard may already
        // be gone (review C5).
        let _ = std::fs::remove_file(dst);
        return Err(e);
    }
    if cancel.is_cancelled() {
        let _ = std::fs::remove_file(dst);
        return Err(ConvertError::Cancelled);
    }
    Ok(info)
}

/// The image's EXIF as raw TIFF with orientation reset to 1, if any.
fn read_exif(handle: &libheif_rs::ImageHandle) -> Option<Vec<u8>> {
    let mut ids: [ItemId; 4] = [0; 4];
    let n = handle.metadata_block_ids(&mut ids, b"Exif");
    let block = handle.metadata(*ids[..n].first()?).ok()?;
    let mut tiff = exif::tiff_from_heif_exif(&block)?.to_vec();
    exif::reset_orientation(&mut tiff);
    Some(tiff)
}

/// Checks a written JPEG by fully decoding it (PLAN §7.4). Returns its size.
pub fn verify_jpeg(path: &Path) -> Result<(u32, u32), ConvertError> {
    let img = image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
        .map_err(|e| ConvertError::Verify(format!("JPEG does not decode: {e}")))?;
    Ok((img.width(), img.height()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use libheif_rs::{Channel, CompressionFormat, EncoderQuality};

    /// A HEIC made here: left half red, right half blue. `w` odd makes
    /// libheif pad its rows, so the repacking path runs too.
    fn make_heic(path: &Path, w: u32, h: u32) {
        let mut img = libheif_rs::Image::new(w, h, ColorSpace::Rgb(RgbChroma::Rgb)).unwrap();
        img.create_plane(Channel::Interleaved, w, h, 8).unwrap();
        {
            let planes = img.planes_mut();
            let p = planes.interleaved.unwrap();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let px = &mut p.data[y * p.stride + x * 3..][..3];
                    px.copy_from_slice(if x < w as usize / 2 {
                        &[220, 20, 20]
                    } else {
                        &[20, 20, 220]
                    });
                }
            }
        }
        let lib = LibHeif::new();
        let mut enc = lib.encoder_for_format(CompressionFormat::Hevc).unwrap();
        enc.set_quality(EncoderQuality::Lossy(90)).unwrap();
        let mut ctx = HeifContext::new().unwrap();
        ctx.encode_image(&img, &mut enc, None).unwrap();
        ctx.write_to_file(path.to_str().unwrap()).unwrap();
    }

    /// Converted without copying the frame first (review P3): same size,
    /// same colours, for packed and padded rows.
    #[test]
    fn heic_converts_to_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        for (w, h) in [(64, 48), (37, 21)] {
            let src = dir.path().join(format!("IMG_{w}.HEIC"));
            let dst = dir.path().join(format!("IMG_{w}.JPG"));
            make_heic(&src, w, h);
            let info = heic_to_jpeg(&src, &dst, DEFAULT_JPEG_QUALITY).unwrap();
            assert_eq!((info.width, info.height), (w, h));
            let out = image::open(&dst).unwrap().to_rgb8();
            assert_eq!(out.dimensions(), (w, h));
            let (left, right) = (out.get_pixel(2, h / 2), out.get_pixel(w - 3, h / 2));
            assert!(left[0] > 150 && left[2] < 100, "{w}: left {left:?}");
            assert!(right[2] > 150 && right[0] < 100, "{w}: right {right:?}");

            // The viewer's smaller copy, shrunk from the borrowed pixels.
            let small = dir.path().join(format!("view_{w}.jpg"));
            let (sw, sh) = crate::display_jpeg(&src, &small, 16).unwrap();
            assert_eq!(sw.max(sh), 16, "{w}");
            assert_eq!(image::open(&small).unwrap().width(), sw);
        }
    }
}
