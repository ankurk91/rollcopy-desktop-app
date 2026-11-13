//! One copy job: read → convert → verify → rename (PLAN §7).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rollcopy_device::{DeviceError, MediaRead};
use rollcopy_library::Asset;
use tempfile::TempPath;
use tokio::sync::mpsc::UnboundedSender;

use crate::layout::Settings;
use crate::queue::{JobId, JobState, QueueEvent, Stage};

/// Emit progress at most this often (bytes).
const PROGRESS_STEP: u64 = 512 * 1024;

pub(crate) enum JobError {
    /// Phone gone, locked out, or the link dropped: pause, don't fail.
    Connection(String),
    Failed {
        stage: Stage,
        error: String,
    },
}

fn failed(stage: Stage, e: impl ToString) -> JobError {
    JobError::Failed {
        stage,
        error: e.to_string(),
    }
}

fn device(stage: Stage, e: DeviceError) -> JobError {
    if e.is_connection_problem() {
        JobError::Connection(e.to_string())
    } else {
        failed(stage, e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Work {
    Copy,
    HeicToJpg,
    VideoToMp4,
}

fn work_for(from: &str, to: &str) -> Work {
    match (from, to) {
        ("HEIC" | "HEIF", "JPG") => Work::HeicToJpg,
        ("MOV", "MP4") => Work::VideoToMp4,
        // Same bytes, at most a renamed extension (JPEG → JPG).
        _ => Work::Copy,
    }
}

pub(crate) struct JobSpec {
    pub id: JobId,
    pub asset: Asset,
    pub dest: PathBuf,
    /// Replace a file at `dest`. Otherwise the job fails rather than touch it.
    pub overwrite: bool,
    pub settings: Settings,
    pub staging_dir: PathBuf,
}

/// `<pid>-<n>`, unique across jobs, queues and Rollcopy processes.
fn unique_tag() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{}-{n}", std::process::id())
}

/// A staging file name no other job uses: `<pid>-<n>.<ext>` (audit REL-03).
fn staging_path(dir: &Path, ext: &str) -> PathBuf {
    dir.join(format!("{}.{ext}", unique_tag()))
}

/// The temporary file a job writes next to its destination:
/// `.rollcopy-<pid>-<n>-<name>.part`. Never shared with another job, even one
/// with the same destination (audit REL-02).
pub(crate) fn part_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().unwrap_or_default().to_string_lossy();
    dest.with_file_name(format!(".rollcopy-{}-{name}.part", unique_tag()))
}

/// Runs a job. On success returns whether an existing file was replaced.
/// Temporary files are removed however the job ends, even when its task is
/// aborted; the destination is only touched by the final rename, so a failed
/// copy never damages an existing file.
pub(crate) async fn run(
    spec: &JobSpec,
    fs: &mut impl MediaRead,
    events: &UnboundedSender<QueueEvent>,
) -> Result<bool, JobError> {
    let plan = spec.asset.save_plan();
    let ext = plan.from_ext.to_ascii_lowercase();
    // Deleted on drop unless persisted.
    let part =
        TempPath::try_from_path(part_path(&spec.dest)).map_err(|e| failed(Stage::Write, e))?;
    let staging = TempPath::try_from_path(staging_path(&spec.staging_dir, &ext))
        .map_err(|e| failed(Stage::Write, e))?;
    run_inner(spec, fs, events, part, &staging).await
}

async fn run_inner(
    spec: &JobSpec,
    fs: &mut impl MediaRead,
    events: &UnboundedSender<QueueEvent>,
    part_guard: TempPath,
    staging: &Path,
) -> Result<bool, JobError> {
    let part: &Path = &part_guard;
    let id = spec.id;
    let state = |s: JobState| {
        let _ = events.send(QueueEvent::State { id, state: s });
    };
    let plan = spec.asset.save_plan();
    let work = work_for(&plan.from_ext, &plan.to_ext);

    // ---- read
    state(JobState::Reading);
    let total = fs
        .stat(&plan.source)
        .await
        .map_err(|e| device(Stage::Read, e))?
        .size;
    if let Some(dir) = spec.dest.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| failed(Stage::Write, e))?;
    }
    tokio::fs::create_dir_all(&spec.staging_dir)
        .await
        .map_err(|e| failed(Stage::Write, e))?;

    let read_to = if work == Work::Copy { part } else { staging };
    let mut last = 0u64;
    let copied = fs
        .copy_to_local(&plan.source, read_to, |done| {
            if done - last >= PROGRESS_STEP || done == total {
                last = done;
                let _ = events.send(QueueEvent::Progress { id, done, total });
            }
        })
        .await
        .map_err(|e| match e {
            DeviceError::LocalIo(io) => failed(Stage::Write, io),
            other => device(Stage::Read, other),
        })?;
    if copied != total {
        return Err(failed(
            Stage::Verify,
            format!("read {copied} bytes, expected {total}"),
        ));
    }

    // ---- convert + verify
    match work {
        Work::Copy => {
            state(JobState::Verifying);
            let len = tokio::fs::metadata(part)
                .await
                .map_err(|e| failed(Stage::Verify, e))?
                .len();
            if len != total {
                return Err(failed(
                    Stage::Verify,
                    format!("wrote {len} bytes, expected {total}"),
                ));
            }
        }
        Work::HeicToJpg => {
            state(JobState::Converting);
            let (src, dst, q) = (
                staging.to_path_buf(),
                part.to_path_buf(),
                spec.settings.jpeg_quality,
            );
            blocking(move || rollcopy_convert::heic_to_jpeg(&src, &dst, q))
                .await
                .map_err(|e| failed(Stage::Convert, e))?;
            state(JobState::Verifying);
            let dst = part.to_path_buf();
            blocking(move || rollcopy_convert::verify_jpeg(&dst))
                .await
                .map_err(|e| failed(Stage::Verify, e))?;
        }
        Work::VideoToMp4 => {
            state(JobState::Converting);
            let (src, dst, mode) = (
                staging.to_path_buf(),
                part.to_path_buf(),
                spec.settings.video_mode,
            );
            let source_info = blocking(move || {
                let info = rollcopy_convert::probe(&src)?;
                rollcopy_convert::to_mp4(&src, &dst, mode)?;
                Ok(info)
            })
            .await
            .map_err(|e| failed(Stage::Convert, e))?;
            state(JobState::Verifying);
            let dst = part.to_path_buf();
            blocking(move || rollcopy_convert::verify_mp4(&dst, source_info.duration_secs))
                .await
                .map_err(|e| failed(Stage::Verify, e))?;
        }
    }

    // ---- finish: capture date as modified time, then the atomic rename.
    if let Some(when) = spec.asset.captured {
        let t = SystemTime::from(when);
        let p = part.to_path_buf();
        blocking(move || {
            std::fs::File::options()
                .write(true)
                .open(&p)
                .and_then(|f| f.set_modified(t))
                .map_err(rollcopy_convert::ConvertError::Io)
        })
        .await
        .map_err(|e| failed(Stage::Write, e))?;
    }
    let dest = spec.dest.clone();
    let overwrite = spec.overwrite;
    tokio::task::spawn_blocking(move || {
        let existed = overwrite && dest.exists();
        let persisted = if overwrite {
            part_guard.persist(&dest)
        } else {
            // Atomic "rename unless something is there" (renameat2 NOREPLACE,
            // or link + unlink), so a file that appeared meanwhile is kept.
            part_guard.persist_noclobber(&dest)
        };
        match persisted {
            Ok(()) => {
                sync_parent(&dest);
                Ok(existed)
            }
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Err(failed(
                Stage::Write,
                "another file appeared at the destination; left it alone",
            )),
            Err(e) => Err(failed(Stage::Write, e.error)),
        }
    })
    .await
    .map_err(|e| failed(Stage::Write, e))?
}

/// Makes a rename in `path`'s folder survive a power cut (audit REL-09).
fn sync_parent(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let _ = std::fs::File::open(dir).and_then(|d| d.sync_all());
    }
    #[cfg(not(unix))]
    let _ = path;
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, rollcopy_convert::ConvertError> + Send + 'static,
) -> Result<T, String> {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e.to_string()),
        Err(join) => Err(join.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_kinds() {
        assert_eq!(work_for("HEIC", "JPG"), Work::HeicToJpg);
        assert_eq!(work_for("MOV", "MP4"), Work::VideoToMp4);
        assert_eq!(work_for("JPEG", "JPG"), Work::Copy);
        assert_eq!(work_for("PNG", "PNG"), Work::Copy);
        assert_eq!(work_for("MP4", "MP4"), Work::Copy);
    }

    #[test]
    fn staging_paths_are_unique() {
        let dir = Path::new("/staging");
        let a = staging_path(dir, "heic");
        let b = staging_path(dir, "heic");
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(&format!("{}-", std::process::id())));
        assert!(name.ends_with(".heic"));
    }

    #[test]
    fn part_paths_are_hidden_unique_and_beside_the_destination() {
        let dest = Path::new("/d/IMG_1.JPG");
        let (a, b) = (part_path(dest), part_path(dest));
        assert_ne!(a, b);
        assert_eq!(a.parent(), dest.parent());
        let name = a.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with(".rollcopy-") && name.ends_with("-IMG_1.JPG.part"));
    }
}
