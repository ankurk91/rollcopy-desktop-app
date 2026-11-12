//! Minimal EXIF handling for HEIC → JPG: unwrap HEIF's EXIF block and reset
//! the orientation tag. libheif already rotates the pixels while decoding,
//! so keeping the original orientation would make viewers rotate twice.

const ORIENTATION_TAG: u16 = 0x0112;
const TYPE_SHORT: u16 = 3;

/// HEIF stores EXIF as a 4-byte big-endian offset to the TIFF header,
/// optionally followed by an `Exif\0\0` prefix. Returns the raw TIFF data
/// (starting with `II*\0` or `MM\0*`), or `None` if it doesn't look valid.
pub fn tiff_from_heif_exif(block: &[u8]) -> Option<&[u8]> {
    let offset = u32::from_be_bytes(block.get(..4)?.try_into().ok()?) as usize;
    let mut rest = block.get(4 + offset..)?;
    if rest.starts_with(b"Exif\0\0") {
        rest = &rest[6..];
    }
    (rest.starts_with(b"II*\0") || rest.starts_with(b"MM\0*")).then_some(rest)
}

/// Sets the orientation tag in IFD0 to 1 ("normal") if present. Returns
/// whether a tag was changed. Leaves malformed data untouched.
pub fn reset_orientation(tiff: &mut [u8]) -> bool {
    let le = match tiff.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return false,
    };
    let rd16 = |b: &[u8], at: usize| -> Option<u16> {
        let s: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let rd32 = |b: &[u8], at: usize| -> Option<u32> {
        let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(s)
        } else {
            u32::from_be_bytes(s)
        })
    };

    let Some(ifd0) = rd32(tiff, 4).map(|v| v as usize) else {
        return false;
    };
    let Some(count) = rd16(tiff, ifd0) else {
        return false;
    };
    for i in 0..count as usize {
        let entry = ifd0 + 2 + i * 12;
        if rd16(tiff, entry) != Some(ORIENTATION_TAG) {
            continue;
        }
        if rd16(tiff, entry + 2) != Some(TYPE_SHORT) || rd32(tiff, entry + 4) != Some(1) {
            return false;
        }
        let one = if le {
            1u16.to_le_bytes()
        } else {
            1u16.to_be_bytes()
        };
        match tiff.get_mut(entry + 8..entry + 10) {
            Some(v) => {
                v.copy_from_slice(&one);
                return true;
            }
            None => return false,
        }
    }
    false
}

/// Reads the orientation tag (1–8) from IFD0, if present.
pub fn orientation(tiff: &[u8]) -> Option<u16> {
    let le = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let rd16 = |at: usize| -> Option<u16> {
        let s: [u8; 2] = tiff.get(at..at + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(s)
        } else {
            u16::from_be_bytes(s)
        })
    };
    let s: [u8; 4] = tiff.get(4..8)?.try_into().ok()?;
    let ifd0 = if le {
        u32::from_le_bytes(s)
    } else {
        u32::from_be_bytes(s)
    } as usize;
    let count = rd16(ifd0)?;
    (0..count as usize)
        .map(|i| ifd0 + 2 + i * 12)
        .find(|&e| rd16(e) == Some(ORIENTATION_TAG))
        .and_then(|e| rd16(e + 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Big-endian TIFF with IFD0 holding Make (ASCII) and Orientation = 6.
    fn sample_be() -> Vec<u8> {
        let mut t = b"MM\0*".to_vec();
        t.extend(8u32.to_be_bytes()); // IFD0 at 8
        t.extend(2u16.to_be_bytes()); // 2 entries
        // Make: tag 0x010F, ASCII, count 4, inline "App\0"
        t.extend(0x010Fu16.to_be_bytes());
        t.extend(2u16.to_be_bytes());
        t.extend(4u32.to_be_bytes());
        t.extend(b"App\0");
        // Orientation: SHORT, count 1, value 6 (rotate 90 CW)
        t.extend(ORIENTATION_TAG.to_be_bytes());
        t.extend(TYPE_SHORT.to_be_bytes());
        t.extend(1u32.to_be_bytes());
        t.extend(6u16.to_be_bytes());
        t.extend([0, 0]);
        t.extend(0u32.to_be_bytes()); // no next IFD
        t
    }

    #[test]
    fn resets_big_endian_orientation() {
        let mut t = sample_be();
        assert_eq!(orientation(&t), Some(6));
        assert!(reset_orientation(&mut t));
        assert_eq!(orientation(&t), Some(1));
    }

    #[test]
    fn resets_little_endian_orientation() {
        let mut t = b"II*\0".to_vec();
        t.extend(8u32.to_le_bytes());
        t.extend(1u16.to_le_bytes());
        t.extend(ORIENTATION_TAG.to_le_bytes());
        t.extend(TYPE_SHORT.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend(8u16.to_le_bytes());
        t.extend([0, 0]);
        t.extend(0u32.to_le_bytes());
        assert_eq!(orientation(&t), Some(8));
        assert!(reset_orientation(&mut t));
        assert_eq!(orientation(&t), Some(1));
    }

    #[test]
    fn unwraps_heif_block() {
        let tiff = sample_be();
        // Offset 0, then an Exif\0\0 prefix (as some encoders write it).
        let mut block = 6u32.to_be_bytes().to_vec();
        block.extend(b"Exif\0\0");
        block.extend(&tiff);
        assert_eq!(tiff_from_heif_exif(&block), Some(&tiff[..]));
        // Plain offset 0 with TIFF straight after.
        let mut plain = 0u32.to_be_bytes().to_vec();
        plain.extend(&tiff);
        assert_eq!(tiff_from_heif_exif(&plain), Some(&tiff[..]));
        assert_eq!(tiff_from_heif_exif(b"\0\0\0\0junk"), None);
    }

    #[test]
    fn ignores_garbage() {
        let mut junk = b"XX\0\0\0\0\0\0".to_vec();
        assert!(!reset_orientation(&mut junk));
        let mut truncated = sample_be()[..12].to_vec();
        assert!(!reset_orientation(&mut truncated));
    }
}
