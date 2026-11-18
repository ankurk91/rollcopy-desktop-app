//! Format conversion: HEIC → JPG and MOV → MP4, with output checks.
//! No UI dependency; everything here is blocking (run on worker threads).

mod display;
pub mod exif;
mod heic;
mod video;

pub use display::{display_jpeg, image_size, veil_jpeg};
pub use heic::{DEFAULT_JPEG_QUALITY, JpegInfo, heic_to_jpeg, heic_to_jpeg_with, verify_jpeg};
pub use video::{
    VideoInfo, VideoMode, ffmpeg_available, probe, to_mp4, to_mp4_with, to_mp4_with_info,
    verify_mp4,
};

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Set once at quit: every [`Cancel`] reads as cancelled from then on.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
/// External tools (ffmpeg, ffprobe) running right now.
static RUNNING_TOOLS: AtomicUsize = AtomicUsize::new(0);

/// Stops all conversion work for good, for quitting (review C4). Running
/// ffmpeg processes are killed within one poll (20 ms) and remove their
/// output; nothing new starts. A child process isn't killed when its
/// parent exits, so without this an H.264 encode could keep writing in
/// the user's folder long after Rollcopy closed. Waits up to `wait` for
/// the tools to end; `false` if some were still running then.
pub fn shutdown(wait: Duration) -> bool {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + wait;
    while RUNNING_TOOLS.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    RUNNING_TOOLS.load(Ordering::SeqCst) == 0
}

/// Whether [`shutdown`] was called.
pub fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::SeqCst)
}

/// Counts a running tool until dropped (after the process has ended).
pub(crate) struct ToolRunning;

impl ToolRunning {
    pub(crate) fn start() -> Self {
        RUNNING_TOOLS.fetch_add(1, Ordering::SeqCst);
        ToolRunning
    }
}

impl Drop for ToolRunning {
    fn drop(&mut self) {
        RUNNING_TOOLS.fetch_sub(1, Ordering::SeqCst);
    }
}

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

    /// Set for this job, or the whole app is quitting ([`shutdown`]).
    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) || is_shutting_down()
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
