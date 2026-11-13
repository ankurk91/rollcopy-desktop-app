//! Format conversion: HEIC → JPG and MOV → MP4, with output checks.
//! No UI dependency; everything here is blocking (run on worker threads).

mod display;
pub mod exif;
mod heic;
mod video;

pub use display::{display_jpeg, image_size, veil_jpeg};
pub use heic::{DEFAULT_JPEG_QUALITY, JpegInfo, heic_to_jpeg, heic_to_jpeg_with, verify_jpeg};
pub use video::{VideoInfo, VideoMode, ffmpeg_available, probe, to_mp4, to_mp4_with, verify_mp4};

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
    #[error("cancelled")]
    Cancelled,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Stops conversion work that runs outside async code (audit NEW-03):
/// a running ffmpeg is killed, and a HEIC conversion doesn't write (or
/// removes) its output. Cheap to clone; all clones share one flag.
#[derive(Debug, Clone, Default)]
pub struct Cancel(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// `Err(Cancelled)` once cancelled.
    pub fn check(&self) -> Result<(), ConvertError> {
        if self.is_cancelled() {
            Err(ConvertError::Cancelled)
        } else {
            Ok(())
        }
    }
}
