//! Queue behaviour against a fake phone (no device needed).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::{TimeZone, Utc};
use rollcopy_device::{DeviceError, EntryKind, MediaRead, Stat};
use rollcopy_library::{Asset, MediaKind};
use rollcopy_queue::{
    ConflictPolicy, JobId, JobState, Layout, Opener, QueueCommand, QueueEvent, QueueHandle,
    Settings,
};
use tokio::sync::mpsc;

const UDID: &str = "00000000-0000000000000000";

/// A phone holding files in memory. Read-only, like the real thing.
#[derive(Clone, Default)]
struct FakePhone {
    files: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// Reads that fail as if the cable dropped, while the phone stays listed.
    fail_reads: Arc<Mutex<usize>>,
    /// Reads hang (a wedged phone).
    hang: Arc<AtomicBool>,
}

impl FakePhone {
    fn with(files: &[(&str, &[u8])]) -> Self {
        let p = Self::default();
        for (path, data) in files {
            p.files
                .lock()
                .unwrap()
                .insert((*path).to_owned(), data.to_vec());
        }
        p
    }
}

struct FakeMedia(FakePhone);

impl MediaRead for FakeMedia {
    async fn stat(&mut self, path: &str) -> Result<Stat, DeviceError> {
        let files = self.0.files.lock().unwrap();
        let data = files
            .get(path)
            .ok_or_else(|| DeviceError::NotFound(path.to_owned()))?;
        Ok(Stat {
            kind: EntryKind::File,
            size: data.len() as u64,
            modified: SystemTime::UNIX_EPOCH,
        })
    }

    async fn copy_to_local(
        &mut self,
        path: &str,
        dest: &Path,
        mut progress: impl FnMut(u64) + Send,
    ) -> Result<u64, DeviceError> {
        let data = self
            .0
            .files
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| DeviceError::NotFound(path.to_owned()))?;
        {
            let mut fails = self.0.fail_reads.lock().unwrap();
            if *fails > 0 {
                *fails -= 1;
                return Err(DeviceError::Disconnected("cable wiggled".into()));
            }
        }
        if self.0.hang.load(Ordering::SeqCst) {
            // Partly written, then nothing.
            tokio::fs::write(dest, b"half").await?;
            std::future::pending::<()>().await;
        }
        // Slow enough that two jobs overlap.
        tokio::time::sleep(Duration::from_millis(20)).await;
        tokio::fs::write(dest, &data).await?;
        progress(data.len() as u64);
        Ok(data.len() as u64)
    }
}

impl Opener for FakePhone {
    type Media = FakeMedia;
    async fn open(&self, _udid: &str) -> Result<FakeMedia, DeviceError> {
        Ok(FakeMedia(self.clone()))
    }
}

fn photo(dir: &str, name: &str, secs: i64) -> Asset {
    let mut a = Asset::new(dir, name, MediaKind::Photo);
    a.captured = Some(Utc.timestamp_opt(secs, 0).unwrap());
    a
}

struct Harness {
    queue: QueueHandle,
    events: mpsc::UnboundedReceiver<QueueEvent>,
    _staging: tempfile::TempDir,
}

impl Harness {
    fn start(phone: FakePhone, settings: Settings) -> Self {
        let staging = tempfile::tempdir().unwrap();
        let (tx, events) = mpsc::unbounded_channel();
        let queue =
            rollcopy_queue::spawn_with(phone, settings, staging.path().to_path_buf(), None, tx);
        queue.send(QueueCommand::DeviceReady(UDID.to_owned()));
        Self {
            queue,
            events,
            _staging: staging,
        }
    }

    fn add(&self, assets: Vec<Asset>) {
        self.queue.send(QueueCommand::Add {
            udid: UDID.to_owned(),
            assets,
        });
    }

    /// Waits until `n` jobs have finished; returns their final states and
    /// destinations.
    async fn finish(&mut self, n: usize) -> HashMap<JobId, (JobState, PathBuf)> {
        let mut dests = HashMap::new();
        let mut done = HashMap::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while done.len() < n {
                match self.events.recv().await.expect("queue stopped") {
                    QueueEvent::Added { id, dest, .. } | QueueEvent::Retarget { id, dest } => {
                        dests.insert(id, dest);
                    }
                    QueueEvent::State { id, state } if state.is_finished() => {
                        done.insert(id, (state, dests[&id].clone()));
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("jobs didn't finish");
        done
    }
}

/// Audit REL-01/02: `DCIM/100APPLE/IMG_0057.JPG` and `DCIM/110APPLE/IMG_0057.JPG`,
/// taken the same month, copied together.
#[tokio::test]
async fn same_name_photos_are_both_copied() {
    for layout in [Layout::ByDate, Layout::Flat] {
        let dest = tempfile::tempdir().unwrap();
        let phone = FakePhone::with(&[
            ("/DCIM/100APPLE/IMG_0057.JPG", b"first photo"),
            ("/DCIM/110APPLE/IMG_0057.JPG", b"second photo"),
        ]);
        let mut s = Settings::new(dest.path().to_path_buf());
        s.layout = layout;
        s.on_conflict = ConflictPolicy::Overwrite;
        let mut h = Harness::start(phone, s);
        h.add(vec![
            photo("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000),
            photo("DCIM/110APPLE", "IMG_0057.JPG", 1_700_500_000),
        ]);
        let done = h.finish(2).await;
        let mut contents: Vec<Vec<u8>> = done
            .values()
            .map(|(state, path)| {
                assert_eq!(*state, JobState::Done, "{layout:?}");
                std::fs::read(path).unwrap()
            })
            .collect();
        contents.sort();
        assert_eq!(
            contents,
            vec![b"first photo".to_vec(), b"second photo".to_vec()],
            "{layout:?}"
        );
        let paths: Vec<_> = done.values().map(|(_, p)| p.clone()).collect();
        assert_ne!(paths[0], paths[1]);
        // No temporary files left behind.
        let leftovers = walk(dest.path())
            .into_iter()
            .filter(|p| p.to_string_lossy().ends_with(".part"))
            .count();
        assert_eq!(leftovers, 0);
    }
}

/// A file from another tool at the target path survives every policy.
#[tokio::test]
async fn foreign_file_is_never_overwritten() {
    for policy in [
        ConflictPolicy::Overwrite,
        ConflictPolicy::Skip,
        ConflictPolicy::Ask,
    ] {
        let dest = tempfile::tempdir().unwrap();
        let mut s = Settings::new(dest.path().to_path_buf());
        s.layout = Layout::Flat;
        s.on_conflict = policy;
        std::fs::write(dest.path().join("IMG_0057.JPG"), b"not from this phone").unwrap();
        let phone = FakePhone::with(&[("/DCIM/100APPLE/IMG_0057.JPG", b"phone photo")]);
        let mut h = Harness::start(phone, s);
        h.add(vec![photo("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000)]);
        let done = h.finish(1).await;
        let (state, path) = done.into_values().next().unwrap();
        assert_eq!(state, JobState::Done, "{policy:?}");
        assert_eq!(path, dest.path().join("IMG_0057 (2).JPG"));
        assert_eq!(std::fs::read(&path).unwrap(), b"phone photo");
        assert_eq!(
            std::fs::read(dest.path().join("IMG_0057.JPG")).unwrap(),
            b"not from this phone"
        );
    }
}

/// Copying the same item again is a real conflict: Skip keeps it,
/// Overwrite replaces it in place.
#[tokio::test]
async fn recopy_follows_the_conflict_policy() {
    let dest = tempfile::tempdir().unwrap();
    let mut s = Settings::new(dest.path().to_path_buf());
    s.layout = Layout::Flat;
    s.on_conflict = ConflictPolicy::Skip;
    let phone = FakePhone::with(&[("/DCIM/100APPLE/IMG_0057.JPG", b"v1")]);
    let mut h = Harness::start(phone.clone(), s.clone());
    let a = photo("DCIM/100APPLE", "IMG_0057.JPG", 1_700_000_000);
    h.add(vec![a.clone()]);
    assert_eq!(
        h.finish(1).await.into_values().next().unwrap().0,
        JobState::Done
    );

    h.add(vec![a.clone()]);
    let (state, path) = h.finish(1).await.into_values().next().unwrap();
    assert_eq!(state, JobState::Skipped);
    assert_eq!(path, dest.path().join("IMG_0057.JPG"));

    phone
        .files
        .lock()
        .unwrap()
        .insert("/DCIM/100APPLE/IMG_0057.JPG".into(), b"v2".to_vec());
    s.on_conflict = ConflictPolicy::Overwrite;
    h.queue.send(QueueCommand::SetSettings(s));
    h.add(vec![a]);
    let (state, path) = h.finish(1).await.into_values().next().unwrap();
    assert_eq!(state, JobState::Overwritten);
    assert_eq!(path, dest.path().join("IMG_0057.JPG"));
    assert_eq!(std::fs::read(&path).unwrap(), b"v2");
    assert!(!dest.path().join("IMG_0057 (2).JPG").exists());
}

/// Audit REL-04: a read fails while the phone stays listed. The queue
/// pauses, reconnects on its own and finishes, with no new `DeviceReady`.
#[tokio::test]
async fn resumes_after_a_dropped_read_without_a_replug() {
    let dest = tempfile::tempdir().unwrap();
    let mut s = Settings::new(dest.path().to_path_buf());
    s.layout = Layout::Flat;
    let phone = FakePhone::with(&[("/DCIM/100APPLE/IMG_0001.JPG", b"photo")]);
    *phone.fail_reads.lock().unwrap() = 1;
    let mut h = Harness::start(phone, s);
    h.add(vec![photo("DCIM/100APPLE", "IMG_0001.JPG", 1_700_000_000)]);

    let mut paused = false;
    let mut finished = None;
    tokio::time::timeout(Duration::from_secs(10), async {
        while finished.is_none() {
            match h.events.recv().await.unwrap() {
                QueueEvent::Paused { reason: Some(_) } => paused = true,
                QueueEvent::State { state, .. } if state.is_finished() => finished = Some(state),
                _ => {}
            }
        }
    })
    .await
    .expect("queue stayed paused");
    assert!(paused);
    assert_eq!(finished, Some(JobState::Done));
    assert_eq!(
        std::fs::read(dest.path().join("IMG_0001.JPG")).unwrap(),
        b"photo"
    );
}

/// Audit REL-05: a running job can be cancelled; its temp file goes away.
#[tokio::test]
async fn cancel_stops_a_hung_job_and_cleans_up() {
    let dest = tempfile::tempdir().unwrap();
    let mut s = Settings::new(dest.path().to_path_buf());
    s.layout = Layout::Flat;
    let phone = FakePhone::with(&[
        ("/DCIM/100APPLE/IMG_0001.JPG", b"one"),
        ("/DCIM/100APPLE/IMG_0002.JPG", b"two"),
        ("/DCIM/100APPLE/IMG_0003.JPG", b"three"),
    ]);
    phone.hang.store(true, Ordering::SeqCst);
    let mut h = Harness::start(phone, s);
    h.add(vec![
        photo("DCIM/100APPLE", "IMG_0001.JPG", 1_700_000_000),
        photo("DCIM/100APPLE", "IMG_0002.JPG", 1_700_000_100),
        photo("DCIM/100APPLE", "IMG_0003.JPG", 1_700_000_200),
    ]);
    // Wait until both workers are stuck mid-read.
    let parts = |d: &Path| {
        walk(d)
            .iter()
            .filter(|p| p.to_string_lossy().ends_with(".part"))
            .count()
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while parts(dest.path()) < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("jobs never started");

    h.queue.send(QueueCommand::CancelAll);
    let done = h.finish(3).await;
    assert!(
        done.values().all(|(s, _)| *s == JobState::Cancelled),
        "{done:?}"
    );
    // Temp files go with the cancelled tasks.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(walk(dest.path()), Vec::<PathBuf>::new());
}

/// Audit REL-06: not enough space pauses the queue instead of failing items.
#[tokio::test]
async fn full_disk_pauses_instead_of_failing() {
    let dest = tempfile::tempdir().unwrap();
    let mut s = Settings::new(dest.path().to_path_buf());
    s.layout = Layout::Flat;
    let phone = FakePhone::with(&[("/DCIM/100APPLE/IMG_0001.JPG", b"photo")]);
    let mut h = Harness::start(phone, s);
    let mut big = photo("DCIM/100APPLE", "IMG_0001.JPG", 1_700_000_000);
    // Larger than any disk.
    big.original_size = Some(1 << 60);
    h.add(vec![big]);

    let reason = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match h.events.recv().await.unwrap() {
                QueueEvent::Paused { reason: Some(r) } => return r,
                QueueEvent::State { state, .. } if state.is_finished() => {
                    panic!("job finished: {state:?}")
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(reason.contains("disk is full"), "{reason}");
    // Resume tries again, and pauses again while there's still no room.
    h.queue.send(QueueCommand::Resume);
    let again = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let QueueEvent::Paused { reason: Some(r) } = h.events.recv().await.unwrap() {
                return r;
            }
        }
    })
    .await
    .unwrap();
    assert!(again.contains("disk is full"), "{again}");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
