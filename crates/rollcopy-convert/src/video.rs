//! MOV → MP4 with the `ffmpeg` command-line tool (PLAN §7.2).
//!
//! Default is a lossless remux: video and audio streams are copied, Apple's
//! extra data tracks are left out. iPhone video is HDR (HEVC Main 10, HLG,
//! Dolby Vision 8.4); ffmpeg's MP4 muxer drops the Dolby Vision
//! configuration unless `-strict unofficial` is given, so the result is
//! plain HLG HEVC, which more players handle.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::ConvertError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoMode {
    /// Copy streams into MP4 without re-encoding (default).
    #[default]
    Remux,
    /// Re-encode to H.264 + AAC for old devices, tone-mapping HDR to SDR.
    H264,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
    /// HLG or PQ transfer.
    pub hdr: bool,
    pub has_audio: bool,
}

/// SDR conversion that gave correct colours for iPhone HLG video (2026-11-12).
const TONEMAP: &str = "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv,format=yuv420p";

pub fn ffmpeg_available() -> bool {
    tool_ok("ffmpeg") && tool_ok("ffprobe")
}

fn tool_ok(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Time limit for one ffprobe call.
const PROBE_LIMIT: Duration = Duration::from_secs(60);

/// Time limit for converting a video of `secs` seconds (audit REL-05).
/// Generous: a remux runs far faster than real time, and software H.264
/// with tone-mapping took about 3× real time on the test laptop.
fn convert_limit(mode: VideoMode, secs: f64) -> Duration {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    match mode {
        VideoMode::Remux => Duration::from_secs_f64((4.0 * secs).max(120.0)),
        VideoMode::H264 => Duration::from_secs_f64((20.0 * secs).max(600.0)),
    }
}

/// Runs a tool, killing it if it takes longer than `limit`. Returns stdout.
fn run(cmd: &mut Command, what: &str, limit: Duration) -> Result<String, ConvertError> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConvertError::ToolMissing(what.to_owned())
            } else {
                ConvertError::Io(e)
            }
        })?;
    // Drain both pipes on their own threads so a chatty tool can't block.
    let drain = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut r) = r {
                let _ = r.read_to_end(&mut buf);
            }
            buf
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|r| Box::new(r) as Box<dyn Read + Send>),
    );

    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    let Some(status) = status else {
        return Err(ConvertError::Tool {
            tool: what.to_owned(),
            message: format!("stopped after {} s without finishing", limit.as_secs()),
        });
    };
    if !status.success() {
        let err = String::from_utf8_lossy(&err);
        let tail: Vec<&str> = err.lines().rev().take(5).collect();
        return Err(ConvertError::Tool {
            tool: what.to_owned(),
            message: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
        });
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Reads codec, size, duration and HDR flag with `ffprobe`.
pub fn probe(path: &Path) -> Result<VideoInfo, ConvertError> {
    let out = run(
        Command::new("ffprobe")
            .args(["-v", "error", "-show_entries"])
            .arg("format=duration:stream=codec_type,codec_name,width,height,color_transfer")
            .args(["-of", "flat"])
            .arg(path),
        "ffprobe",
        PROBE_LIMIT,
    )?;
    parse_probe(&out)
}

/// Parses `ffprobe -of flat` output (`streams.stream.0.codec_name="hevc"`).
fn parse_probe(out: &str) -> Result<VideoInfo, ConvertError> {
    use std::collections::BTreeMap;
    let mut streams: BTreeMap<usize, BTreeMap<&str, &str>> = BTreeMap::new();
    let mut info = VideoInfo::default();
    for line in out.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim_matches('"');
        if key == "format.duration" {
            info.duration_secs = value.parse().unwrap_or(0.0);
        } else if let Some(rest) = key.strip_prefix("streams.stream.")
            && let Some((idx, field)) = rest.split_once('.')
            && let Ok(idx) = idx.parse::<usize>()
        {
            streams.entry(idx).or_default().insert(field, value);
        }
    }
    info.has_audio = streams
        .values()
        .any(|s| s.get("codec_type") == Some(&"audio"));
    let video = streams
        .values()
        .find(|s| s.get("codec_type") == Some(&"video"))
        .ok_or_else(|| ConvertError::Decode("no video stream".into()))?;
    info.codec = video.get("codec_name").unwrap_or(&"").to_string();
    info.width = video.get("width").and_then(|v| v.parse().ok()).unwrap_or(0);
    info.height = video
        .get("height")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    info.hdr = matches!(
        video.get("color_transfer").copied(),
        Some("arib-std-b67" | "smpte2084")
    );
    Ok(info)
}

/// Converts `src` (MOV, or any ffmpeg-readable video) to an MP4 at `dst`
/// (overwritten). Keeps the creation date and other container metadata.
pub fn to_mp4(src: &Path, dst: &Path, mode: VideoMode) -> Result<(), ConvertError> {
    let info = probe(src)?;
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-y", "-i"])
        .arg(src)
        .args(["-map", "0:v:0", "-map", "0:a?", "-map_metadata", "0"]);
    match mode {
        VideoMode::Remux => {
            cmd.args(["-c", "copy"]);
            // Apple players need `hvc1`, not `hev1`, for HEVC in MP4.
            match info.codec.as_str() {
                "hevc" => cmd.args(["-tag:v", "hvc1"]),
                "h264" => cmd.args(["-tag:v", "avc1"]),
                _ => &mut cmd,
            };
        }
        VideoMode::H264 => {
            if info.hdr {
                cmd.args(["-vf", TONEMAP]);
            } else {
                cmd.args(["-pix_fmt", "yuv420p"]);
            }
            cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", "18"])
                .args(["-c:a", "aac", "-b:a", "192k"]);
        }
    }
    cmd.args(["-movflags", "+faststart+use_metadata_tags", "-f", "mp4"])
        .arg(dst);
    run(&mut cmd, "ffmpeg", convert_limit(mode, info.duration_secs))?;
    Ok(())
}

/// Checks a converted video (PLAN §7.4): it must probe, have a video stream,
/// and last as long as the source (within half a second).
pub fn verify_mp4(path: &Path, expected_secs: f64) -> Result<VideoInfo, ConvertError> {
    let info = probe(path)?;
    if info.duration_secs <= 0.0 || (info.duration_secs - expected_secs).abs() > 0.5 {
        return Err(ConvertError::Verify(format!(
            "duration {:.2}s, expected {:.2}s",
            info.duration_secs, expected_secs
        )));
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_hung_tool_is_stopped() {
        let start = Instant::now();
        let err = run(
            Command::new("sleep").arg("30"),
            "sleep",
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(err.to_string().contains("without finishing"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn output_and_failures_come_back() {
        let out = run(
            Command::new("sh").args(["-c", "echo hi; echo oops >&2"]),
            "sh",
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(out.trim(), "hi");
        let err = run(
            Command::new("sh").args(["-c", "echo bad >&2; exit 3"]),
            "sh",
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(err.to_string().contains("bad"), "{err}");
    }

    #[test]
    fn limits_scale_with_length() {
        assert_eq!(
            convert_limit(VideoMode::Remux, 10.0),
            Duration::from_secs(120)
        );
        assert_eq!(
            convert_limit(VideoMode::Remux, 60.0),
            Duration::from_secs(240)
        );
        assert_eq!(
            convert_limit(VideoMode::H264, 60.0),
            Duration::from_secs(1200)
        );
        assert_eq!(
            convert_limit(VideoMode::H264, f64::NAN),
            Duration::from_secs(600)
        );
    }

    #[test]
    fn parses_flat_probe() {
        // Shape of ffprobe 6.1 output for an iPhone MOV (2026-11-12).
        let out = r#"streams.stream.0.codec_name="hevc"
streams.stream.0.codec_type="video"
streams.stream.0.width=1920
streams.stream.0.height=1080
streams.stream.0.color_transfer="arib-std-b67"
streams.stream.1.codec_name="aac"
streams.stream.1.codec_type="audio"
streams.stream.2.codec_type="data"
format.duration="15.335000"
"#;
        let i = parse_probe(out).unwrap();
        assert_eq!(i.codec, "hevc");
        assert_eq!((i.width, i.height), (1920, 1080));
        assert!(i.hdr && i.has_audio);
        assert!((i.duration_secs - 15.335).abs() < 1e-9);
        assert!(parse_probe("format.duration=\"1\"\n").is_err());
    }
}
