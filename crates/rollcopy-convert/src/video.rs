//! MOV → MP4 with the `ffmpeg` command-line tool (PLAN §7.2).
//!
//! Default is a lossless remux: video and audio streams are copied, Apple's
//! extra data tracks are left out. iPhone video is HDR (HEVC Main 10, HLG,
//! Dolby Vision 8.4); ffmpeg's MP4 muxer drops the Dolby Vision
//! configuration unless `-strict unofficial` is given, so the result is
//! plain HLG HEVC, which more players handle.

use std::path::Path;
use std::process::Command;

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

fn run(cmd: &mut Command, what: &str) -> Result<String, ConvertError> {
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ConvertError::ToolMissing(what.to_owned())
        } else {
            ConvertError::Io(e)
        }
    })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(5).collect();
        return Err(ConvertError::Tool {
            tool: what.to_owned(),
            message: tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
    run(&mut cmd, "ffmpeg")?;
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
