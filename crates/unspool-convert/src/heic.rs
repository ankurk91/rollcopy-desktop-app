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

/// Converts the primary image of a HEIC/HEIF file to a JPEG at `dst`
/// (overwritten). Blocking and CPU-heavy: run it on a worker thread.
pub fn heic_to_jpeg(src: &Path, dst: &Path, quality: u8) -> Result<JpegInfo, ConvertError> {
    let src_str = src
        .to_str()
        .ok_or_else(|| ConvertError::Decode("source path is not UTF-8".into()))?;
    let ctx = HeifContext::read_from_file(src_str).map_err(heif_err)?;
    let handle = ctx.primary_image_handle().map_err(heif_err)?;

    let lib = LibHeif::new();
    let decoded = lib
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), None)
        .map_err(heif_err)?;
    let plane = decoded
        .planes()
        .interleaved
        .ok_or_else(|| ConvertError::Decode("no interleaved RGB plane".into()))?;
    let (w, h) = (plane.width, plane.height);
    let row = w as usize * 3;
    let mut rgb = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        let start = y * plane.stride;
        rgb.extend_from_slice(
            plane
                .data
                .get(start..start + row)
                .ok_or_else(|| ConvertError::Decode("short pixel row".into()))?,
        );
    }

    let exif = read_exif(&handle);
    let icc = handle.color_profile_raw().map(|p| p.data);
    let info = JpegInfo {
        width: w,
        height: h,
        has_exif: exif.is_some(),
        has_icc: icc.is_some(),
    };

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
    enc.write_image(&rgb, w, h, ExtendedColorType::Rgb8)
        .map_err(|e| ConvertError::Encode(e.to_string()))?;
    out.flush()?;
    out.into_inner()
        .map_err(|e| ConvertError::Io(e.into_error()))?
        .sync_all()?;
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
