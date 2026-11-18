//! Quitting stops conversions (review C4). In its own test binary: the
//! shutdown flag is process-wide and would cancel the other tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use rollcopy_convert::{ConvertError, VideoMode, ffmpeg_available, shutdown, to_mp4_with};

/// ffmpeg processes whose command line mentions `path`.
fn ffmpeg_writing(path: &Path) -> usize {
    let needle = path.to_string_lossy().into_owned();
    std::fs::read_dir("/proc")
        .unwrap()
        .flatten()
        .filter_map(|e| std::fs::read(e.path().join("cmdline")).ok())
        .map(|c| String::from_utf8_lossy(&c).replace('\0', " "))
        .filter(|c| c.starts_with("ffmpeg") && c.contains(&needle))
        .count()
}

#[test]
fn shutdown_kills_ffmpeg_and_blocks_new_work() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("IMG_0001.MOV");
    let ok = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-y"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=30:size=1280x720:rate=30",
        ])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&src)
        .status()
        .unwrap()
        .success();
    assert!(ok, "couldn't generate the test clip");

    // A slow H.264 encode, as the app does for "old devices".
    let dst = dir.path().join(".rollcopy-1-1-IMG_0001.MP4.part");
    let (s, d) = (src.clone(), dst.clone());
    let worker = std::thread::spawn(move || {
        to_mp4_with(&s, &d, VideoMode::H264, &rollcopy_convert::Cancel::new())
    });
    let started = Instant::now();
    while ffmpeg_writing(&dst) == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "ffmpeg never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(shutdown(Duration::from_secs(2)), "tools still running");
    assert_eq!(ffmpeg_writing(&dst), 0);
    assert!(matches!(
        worker.join().unwrap(),
        Err(ConvertError::Cancelled)
    ));
    assert!(!dst.exists(), "half-written output left behind");

    // Nothing new starts after quitting.
    let again = dir.path().join("again.mp4");
    let r = to_mp4_with(
        &src,
        &again,
        VideoMode::Remux,
        &rollcopy_convert::Cancel::new(),
    );
    assert!(matches!(r, Err(ConvertError::Cancelled)), "{r:?}");
    assert!(!again.exists());
}
