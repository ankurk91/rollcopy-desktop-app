//! Format conversion: HEIC → JPG and MOV → MP4, with output checks.
//! No UI dependency; everything here is blocking (run on worker threads).

pub mod exif;
mod heic;
mod video;

pub use heic::{DEFAULT_JPEG_QUALITY, JpegInfo, heic_to_jpeg, verify_jpeg};
pub use video::{VideoInfo, VideoMode, ffmpeg_available, probe, to_mp4, verify_mp4};

#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    #[error("could not read the source: {0}")]
    Decode(String),
    #[error("could not write the converted file: {0}")]
    Encode(String),
    #[error("{tool} failed: {message}")]
    Tool { tool: String, message: String },
    #[error("{0} is not installed")]
    ToolMissing(String),
    #[error("check failed: {0}")]
    Verify(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
