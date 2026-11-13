//! One copy job: read → convert → verify → rename (PLAN §7).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rollcopy_device::{DeviceError, MediaRead};
use rollcopy_library::Asset;
use tempfile::TempPath;
use tokio::sync::mpsc::UnboundedSender;

use crate::layout::Settings;
use crate::queue::{JobId, JobState, QueueEvent, Stage};

/// Emit progress at most this often (audit PERF-04); the last byte count is
/// always sent.
const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

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

/// Deletes `.rollcopy-<pid>-…part` files in `dir` left by a Rollcopy that
/// crashed or was killed (audit REL-09). Files of a process that is still
/// running are kept. Each folder is checked once per session.
pub(crate) fn sweep_orphans(dir: &Path) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SWEPT: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    {
        let mut swept = SWEPT.lock().unwrap_or_else(|e| e.into_inner());
        if !swept
            .get_or_insert_with(HashSet::new)
            .insert(dir.to_path_buf())
        {
            return;
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if let Some(pid) = orphan_pid(&name)
            && pid != std::process::id()
            && !process_alive(pid)
        {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The process id in a `.rollcopy-<pid>-<n>-<name>.part` file name.
fn orphan_pid(name: &str) -> Option<u32> {
    let rest = name.strip_prefix(".rollcopy-")?;
    if !name.ends_with(".part") {
        return None;
    }
    rest.split('-').next()?.parse().ok()
}

fn process_alive(pid: u32) -> bool {
    if cfg!(target_os = "linux") {
        Path::new(&format!("/proc/{pid}")).exists()
    } else {
        // Can't tell; keep the file.
        true
    }
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
        let dir = dir.to_path_buf();
        let _ = tokio::task::spawn_blocking(move || sweep_orphans(&dir)).await;
    }
    tokio::fs::create_dir_all(&spec.staging_dir)
        .await
        .map_err(|e| failed(Stage::Write, e))?;

    let read_to = if work == Work::Copy { part } else { staging };
    let mut last = std::time::Instant::now();
    let copied = fs
        .copy_to_local(&plan.source, read_to, |done| {
            if last.elapsed() >= PROGRESS_EVERY || done == total {
                last = std::time::Instant::now();
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
        let persisted = persist_into(
            part_guard,
            &dest,
            overwrite,
            |p, d| p.persist_noclobber(d),
            |p, d| p.persist(d),
        );
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

/// Moves the finished temporary file to `dest`.
///
/// Without `overwrite` this is atomic "rename unless something is there"
/// (renameat2 NOREPLACE, or link + unlink), so a file that appeared
/// meanwhile is kept. Some filesystems support neither: GVFS FUSE shares
/// opened in Files (`/run/user/…/gvfs/smb-share:…`), sshfs without hard
/// links, some exFAT/NAS FUSE drivers. There every copy used to fail, so
/// if nothing is at `dest` it falls back to a plain rename (audit NEW-01).
/// That isn't atomic, but the queue's reservation already keeps two
/// Rollcopy jobs off the same file. An existing file is never replaced.
///
/// The two steps are passed in so tests can stand in for a filesystem.
fn persist_into(
    part: TempPath,
    dest: &Path,
    overwrite: bool,
    noclobber: impl FnOnce(TempPath, &Path) -> Result<(), tempfile::PathPersistError>,
    replace: impl FnOnce(TempPath, &Path) -> Result<(), tempfile::PathPersistError>,
) -> Result<(), tempfile::PathPersistError> {
    if overwrite {
        return replace(part, dest);
    }
    match noclobber(part, dest) {
        Err(e) if no_atomic_rename(&e.error) => {
            match std::fs::symlink_metadata(dest) {
                Err(m) if m.kind() == std::io::ErrorKind::NotFound => {
                    note_fallback(dest);
                    replace(e.path, dest)
                }
                // Something is there (or can't be checked): leave it.
                Ok(_) => Err(tempfile::PathPersistError {
                    error: std::io::Error::from(std::io::ErrorKind::AlreadyExists),
                    path: e.path,
                }),
                Err(_) => Err(e),
            }
        }
        other => other,
    }
}

/// The filesystem can't do "rename unless something is there".
fn no_atomic_rename(e: &std::io::Error) -> bool {
    // Linux errno values: EPERM, EXDEV, ENOSYS, EOPNOTSUPP.
    matches!(
        e.kind(),
        std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
    ) || matches!(e.raw_os_error(), Some(1 | 18 | 38 | 95))
}

/// Logs the plain-rename fallback once per destination folder.
fn note_fallback(dest: &Path) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static NOTED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let Some(dir) = dest.parent() else {
        return;
    };
    let mut noted = NOTED.lock().unwrap_or_else(|e| e.into_inner());
    if noted
        .get_or_insert_with(HashSet::new)
        .insert(dir.to_path_buf())
    {
        eprintln!(
            "rollcopy: this filesystem can't rename without replacing; \
             using a plain rename for new files"
        );
    }
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

    fn unsupported(part: TempPath) -> Result<(), tempfile::PathPersistError> {
        Err(tempfile::PathPersistError {
            error: std::io::Error::from(std::io::ErrorKind::Unsupported),
            path: part,
        })
    }

    /// A filesystem without RENAME_NOREPLACE or hard links (GVFS, sshfs).
    #[test]
    fn falls_back_to_a_plain_rename_when_nothing_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("IMG_0001.JPG");
        let part = dir.path().join(".rollcopy-1-1-IMG_0001.JPG.part");
        std::fs::write(&part, b"new").unwrap();
        persist_into(
            TempPath::try_from_path(&part).unwrap(),
            &dest,
            false,
            |p, _| unsupported(p),
            |p, d| p.persist(d),
        )
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert!(!part.exists());
    }

    #[test]
    fn fallback_never_touches_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("IMG_0001.JPG");
        let part = dir.path().join(".rollcopy-1-1-IMG_0001.JPG.part");
        std::fs::write(&dest, b"old").unwrap();
        std::fs::write(&part, b"new").unwrap();
        let err = persist_into(
            TempPath::try_from_path(&part).unwrap(),
            &dest,
            false,
            |p, _| unsupported(p),
            |p, d| p.persist(d),
        )
        .unwrap_err();
        assert_eq!(err.error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        drop(err);
        // The temporary file is cleaned up with the error's TempPath.
        assert!(!part.exists());
    }

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
    fn orphans_of_dead_processes_are_swept() {
        let dir = tempfile::tempdir().unwrap();
        // A pid that can't be running (above the Linux maximum).
        let dead = dir.path().join(".rollcopy-99999999-1-IMG_1.JPG.part");
        let ours = part_path(&dir.path().join("IMG_2.JPG"));
        let photo = dir.path().join("IMG_3.JPG");
        let lookalike = dir.path().join(".rollcopy-notes.txt");
        for p in [&dead, &ours, &photo, &lookalike] {
            std::fs::write(p, b"x").unwrap();
        }
        sweep_orphans(dir.path());
        assert!(!dead.exists());
        assert!(ours.exists() && photo.exists() && lookalike.exists());
        assert_eq!(orphan_pid(".rollcopy-42-7-IMG_1.JPG.part"), Some(42));
        assert_eq!(orphan_pid("IMG_1.JPG.part"), None);
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
