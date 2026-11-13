//! MOV → MP4 with the `ffmpeg` command-line tool (PLAN §7.2).
//!
//! Default is a lossless remux: video and audio streams are copied, Apple's
//! extra data tracks are left out. iPhone video is HDR (HEVC Main 10, HLG,
//! Dolby Vision 8.4); ffmpeg's MP4 muxer drops the Dolby Vision
//! configuration unless `-strict unofficial` is given, so the result is
//! plain HLG HEVC, which more players handle.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::{Cancel, ConvertError};

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
    run_with(cmd, what, limit, &Cancel::new())
}

/// [`run`] that also kills the tool as soon as `cancel` is set.
fn run_with(
    cmd: &mut Command,
    what: &str,
    limit: Duration,
    cancel: &Cancel,
) -> Result<String, ConvertError> {
    cancel.check()?;
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
    let mut cancelled = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        cancelled = cancel.is_cancelled();
        if cancelled || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    if cancelled {
        return Err(ConvertError::Cancelled);
    }
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
    to_mp4_with(src, dst, mode, &Cancel::new())
}

/// [`to_mp4`] that kills ffmpeg as soon as `cancel` is set and removes
/// whatever it wrote (audit NEW-03).
pub fn to_mp4_with(
    src: &Path,
    dst: &Path,
    mode: VideoMode,
    cancel: &Cancel,
) -> Result<(), ConvertError> {
    let result = convert(src, dst, mode, cancel);
    if cancel.is_cancelled() {
        let _ = std::fs::remove_file(dst);
        return Err(ConvertError::Cancelled);
    }
    result
}

fn convert(src: &Path, dst: &Path, mode: VideoMode, cancel: &Cancel) -> Result<(), ConvertError> {
    let info = probe(src)?;
    if mode == VideoMode::H264
        && let Some(device) = gpu_device()
    {
        // A clip the GPU path can't take is done in software below.
        match run_h264(src, dst, &info, Encoder::Gpu(device), cancel) {
            Ok(()) => return Ok(()),
            Err(ConvertError::Cancelled) => return Err(ConvertError::Cancelled),
            Err(_) => {}
        }
    }
    match mode {
        VideoMode::Remux => run_remux(src, dst, &info, cancel),
        VideoMode::H264 => run_h264(src, dst, &info, Encoder::Cpu, cancel),
    }
}

/// ffmpeg, quiet unless something fails, never reading stdin.
fn ffmpeg() -> Command {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-v", "error", "-y"]);
    cmd
}

fn finish(cmd: &mut Command, dst: &Path) {
    cmd.args(["-movflags", "+faststart+use_metadata_tags", "-f", "mp4"])
        .arg(dst);
}

fn run_remux(
    src: &Path,
    dst: &Path,
    info: &VideoInfo,
    cancel: &Cancel,
) -> Result<(), ConvertError> {
    let mut cmd = ffmpeg();
    cmd.arg("-i")
        .arg(src)
        .args(["-map", "0:v:0", "-map", "0:a?", "-map_metadata", "0"])
        .args(["-c", "copy"]);
    // Apple players need `hvc1`, not `hev1`, for HEVC in MP4.
    match info.codec.as_str() {
        "hevc" => cmd.args(["-tag:v", "hvc1"]),
        "h264" => cmd.args(["-tag:v", "avc1"]),
        _ => &mut cmd,
    };
    finish(&mut cmd, dst);
    run_with(
        &mut cmd,
        "ffmpeg",
        convert_limit(VideoMode::Remux, info.duration_secs),
        cancel,
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
enum Encoder {
    /// libx264, tone-mapping with zscale (slow: float maths on the CPU).
    Cpu,
    /// Tone-mapping on the GPU with libplacebo (Vulkan) and encoding with
    /// VA-API on this render node. About 40% faster with a tenth of the CPU
    /// on a 1080p HLG iPhone clip (Intel, 2026-11-13); same Hable curve, so
    /// it looks like the software result.
    Gpu(PathBuf),
}

fn run_h264(
    src: &Path,
    dst: &Path,
    info: &VideoInfo,
    enc: Encoder,
    cancel: &Cancel,
) -> Result<(), ConvertError> {
    let mut cmd = ffmpeg();
    if let Encoder::Gpu(device) = &enc {
        cmd.args(["-init_hw_device", "vulkan=vk", "-init_hw_device"])
            .arg(format!("vaapi=va:{}", device.display()))
            .args(["-filter_hw_device", "va"]);
    }
    cmd.arg("-i")
        .arg(src)
        .args(["-map", "0:v:0", "-map", "0:a?", "-map_metadata", "0"]);
    match (&enc, info.hdr) {
        (Encoder::Cpu, true) => {
            cmd.args(["-vf", TONEMAP]);
        }
        (Encoder::Cpu, false) => {
            cmd.args(["-pix_fmt", "yuv420p"]);
        }
        (Encoder::Gpu(_), true) => {
            cmd.args(["-vf", GPU_TONEMAP]);
        }
        (Encoder::Gpu(_), false) => {
            cmd.args(["-vf", "format=nv12,hwupload"]);
        }
    }
    match enc {
        Encoder::Cpu => cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", "18"]),
        // QP 24 came out smaller than libx264 CRF 18 at the same SSIM.
        Encoder::Gpu(_) => cmd.args(["-c:v", "h264_vaapi", "-rc_mode", "CQP", "-qp", "24"]),
    };
    cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    finish(&mut cmd, dst);
    run_with(
        &mut cmd,
        "ffmpeg",
        convert_limit(VideoMode::H264, info.duration_secs),
        cancel,
    )?;
    Ok(())
}

/// HLG/PQ → SDR on the GPU, then onto a VA-API surface for the encoder.
/// (`tonemap_vaapi` only handles HDR10 with mastering metadata, which
/// iPhone HLG video doesn't have; libplacebo handles HLG.)
const GPU_TONEMAP: &str = "libplacebo=tonemapping=hable:colorspace=bt709:\
color_primaries=bt709:color_trc=bt709:range=tv:format=nv12,hwupload";

/// The render node to use for GPU encoding, if a quick trial encode works
/// there. Checked once per run; `ROLLCOPY_NO_GPU` turns it off.
fn gpu_device() -> Option<PathBuf> {
    static DEVICE: OnceLock<Option<PathBuf>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            if std::env::var_os("ROLLCOPY_NO_GPU").is_some() {
                return None;
            }
            let mut nodes: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with("renderD"))
                })
                .collect();
            nodes.sort();
            nodes.into_iter().find(|n| gpu_trial(n))
        })
        .clone()
}

/// Converts a few frames of generated 10-bit HLG video on `device` the way
/// [`run_h264`] would.
fn gpu_trial(device: &Path) -> bool {
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-nostdin",
        "-v",
        "error",
        "-init_hw_device",
        "vulkan=vk",
        "-init_hw_device",
    ])
    .arg(format!("vaapi=va:{}", device.display()))
    .args(["-filter_hw_device", "va", "-f", "lavfi", "-i"])
    .arg(
        "color=c=gray:s=256x144:d=0.2,format=yuv420p10le,\
             setparams=color_primaries=bt2020:color_trc=arib-std-b67:colorspace=bt2020nc",
    )
    .args(["-vf", GPU_TONEMAP])
    .args(["-c:v", "h264_vaapi", "-f", "null", "-"]);
    run(&mut cmd, "ffmpeg", Duration::from_secs(20)).is_ok()
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
    /// Audit NEW-03: cancelling kills a running tool at once.
    #[cfg(unix)]
    #[test]
    fn cancel_kills_the_tool() {
        let cancel = Cancel::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            c.cancel();
        });
        let start = Instant::now();
        let err = run_with(
            Command::new("sleep").arg("30"),
            "sleep",
            Duration::from_secs(60),
            &cancel,
        )
        .unwrap_err();
        let took = start.elapsed();
        assert!(matches!(err, ConvertError::Cancelled), "{err}");
        assert!(took < Duration::from_millis(300), "{took:?}");
    }

    /// A cancelled conversion leaves no output behind.
    #[cfg(unix)]
    #[test]
    fn cancelled_conversion_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let Some(src) = hlg_clip(dir.path()) else {
            eprintln!("ffmpeg with libx265 not available; skipping");
            return;
        };
        let dst = dir.path().join(".rollcopy-1-1-IMG_0001.MP4.part");
        let cancel = Cancel::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            c.cancel();
        });
        let err = to_mp4_with(&src, &dst, VideoMode::H264, &cancel).unwrap_err();
        assert!(matches!(err, ConvertError::Cancelled), "{err}");
        assert!(!dst.exists());
    }

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

    /// 2 s of generated 10-bit HLG HEVC with sound, like an iPhone clip.
    fn hlg_clip(dir: &Path) -> Option<PathBuf> {
        if !ffmpeg_available() {
            return None;
        }
        let src = dir.join("IMG_0001.MOV");
        let ok = Command::new("ffmpeg")
            .args(["-nostdin", "-v", "error", "-y"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "testsrc2=duration=2:size=640x360:rate=30",
            ])
            .args(["-f", "lavfi", "-i", "sine=duration=2"])
            .args([
                "-c:v",
                "libx265",
                "-preset",
                "ultrafast",
                "-pix_fmt",
                "yuv420p10le",
            ])
            .args([
                "-x265-params",
                "colorprim=bt2020:transfer=arib-std-b67:colormatrix=bt2020nc:log-level=error",
            ])
            .args(["-c:a", "aac", "-shortest"])
            .arg(&src)
            .status()
            .ok()?
            .success();
        ok.then_some(src)
    }

    fn check_h264(enc: Encoder) {
        let dir = tempfile::tempdir().unwrap();
        let Some(src) = hlg_clip(dir.path()) else {
            eprintln!("ffmpeg with libx265 not available; skipping");
            return;
        };
        let info = probe(&src).unwrap();
        assert!(info.hdr);
        let dst = dir.path().join(".rollcopy-1-1-IMG_0001.MP4.part");
        run_h264(&src, &dst, &info, enc.clone(), &Cancel::new()).unwrap();
        let out = verify_mp4(&dst, info.duration_secs).unwrap();
        assert_eq!(out.codec, "h264", "{enc:?}");
        assert!(!out.hdr, "{enc:?} left HDR in");
        assert!(out.has_audio);
    }

    #[test]
    fn h264_in_software() {
        check_h264(Encoder::Cpu);
    }

    #[test]
    fn h264_on_the_gpu_when_there_is_one() {
        match gpu_device() {
            Some(device) => check_h264(Encoder::Gpu(device)),
            None => eprintln!("no usable GPU; skipping"),
        }
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
