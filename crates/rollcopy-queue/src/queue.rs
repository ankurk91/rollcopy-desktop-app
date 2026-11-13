//! The transfer queue (PLAN §6 bottom panel, §7, §8): schedules copy jobs on
//! a tokio runtime, asks about conflicts, pauses on disconnects, retries
//! once, and records failures.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use rollcopy_device::{DeviceError, MediaFs, MediaRead, Session};
use rollcopy_library::Asset;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::history::{FailureRecord, History};
use crate::job::{self, JobError, JobSpec};
use crate::layout::{ConflictPolicy, Settings, Target, output_name, resolve_dest};

pub type JobId = u64;

/// Jobs that run at once. Each holds one AFC connection and does its own
/// conversion, so this also caps ffmpeg processes (PLAN §7.6).
const WORKERS: usize = 2;
/// Automatic retries for a real per-file failure before it is left to the user.
const AUTO_RETRIES: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Read,
    Convert,
    Write,
    Verify,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Read => "read",
            Stage::Convert => "convert",
            Stage::Write => "write",
            Stage::Verify => "verify",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Queued,
    Reading,
    Converting,
    Verifying,
    Done,
    /// Done, replacing a file that was already there.
    Overwritten,
    Skipped,
    Failed {
        stage: Stage,
        error: String,
    },
    /// Waiting for the user to choose Skip or Overwrite.
    Conflict,
}

impl JobState {
    pub fn is_finished(&self) -> bool {
        matches!(
            self,
            JobState::Done | JobState::Overwritten | JobState::Skipped | JobState::Failed { .. }
        )
    }
}

#[derive(Debug, Clone)]
pub enum QueueCommand {
    /// Queue these items from this phone. Recently Deleted items and items
    /// already queued are ignored.
    Add {
        udid: String,
        assets: Vec<Asset>,
    },
    /// Settings for jobs added from now on.
    SetSettings(Settings),
    /// Answer for a conflict; `apply_to_all` answers the rest the same way.
    Resolve {
        id: JobId,
        overwrite: bool,
        apply_to_all: bool,
    },
    Retry(JobId),
    RetryAllFailed,
    Pause,
    Resume,
    /// The phone with this UDID is connected and trusted.
    DeviceReady(String),
    DeviceGone,
    /// Forget finished jobs (Done/Skipped/Overwritten).
    ClearFinished,
}

#[derive(Debug, Clone)]
pub enum QueueEvent {
    Added {
        id: JobId,
        name: String,
        source: String,
        dest: PathBuf,
    },
    State {
        id: JobId,
        state: JobState,
    },
    Progress {
        id: JobId,
        done: u64,
        total: u64,
    },
    /// `Some(reason)`: paused; `None`: running again.
    Paused {
        reason: Option<String>,
    },
    /// A job's destination changed: the name it had was taken by the time it
    /// started, so it goes to `IMG_0057 (2).JPG` or similar instead.
    Retarget {
        id: JobId,
        dest: PathBuf,
    },
    /// The first unanswered conflict (PLAN §7.5 dialog).
    Conflict {
        id: JobId,
        name: String,
        dest: PathBuf,
        existing_size: u64,
        existing_modified: Option<SystemTime>,
        new_size: Option<u64>,
        /// Further queued items whose destination also exists.
        others: usize,
    },
    /// No conflict is waiting any more.
    ConflictCleared,
    /// Jobs were removed by `ClearFinished`.
    Cleared {
        ids: Vec<JobId>,
    },
}

#[derive(Clone)]
pub struct QueueHandle {
    tx: UnboundedSender<QueueCommand>,
}

impl QueueHandle {
    pub fn send(&self, cmd: QueueCommand) {
        let _ = self.tx.send(cmd);
    }
}

/// Opens a read-only connection to a phone's media. The real one goes through
/// usbmuxd; tests use a fake phone.
pub trait Opener: Send + Sync + 'static {
    type Media: MediaRead + 'static;
    fn open(&self, udid: &str) -> impl Future<Output = Result<Self::Media, DeviceError>> + Send;
}

/// Opens AFC on a USB-connected phone.
pub struct DeviceOpener;

impl Opener for DeviceOpener {
    type Media = MediaFs;
    async fn open(&self, udid: &str) -> Result<MediaFs, DeviceError> {
        Session::open(udid).await?.media().await
    }
}

/// Starts the queue manager on the current tokio runtime.
pub fn spawn(
    settings: Settings,
    staging_dir: PathBuf,
    history: Option<History>,
    events: UnboundedSender<QueueEvent>,
) -> QueueHandle {
    spawn_with(DeviceOpener, settings, staging_dir, history, events)
}

/// [`spawn`] with another way to reach the phone.
pub fn spawn_with<O: Opener>(
    opener: O,
    settings: Settings,
    staging_dir: PathBuf,
    history: Option<History>,
    events: UnboundedSender<QueueEvent>,
) -> QueueHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let manager = Manager {
        opener: Arc::new(opener),
        reserved: HashMap::new(),
        settings,
        staging_dir,
        history,
        events,
        jobs: HashMap::new(),
        pending: VecDeque::new(),
        running: HashSet::new(),
        conflicts: VecDeque::new(),
        shown_conflict: None,
        conflict_default: None,
        conns: Vec::new(),
        user_paused: false,
        device_paused: None,
        ready_udid: None,
        next_id: 1,
    };
    tokio::spawn(manager.run(rx));
    QueueHandle { tx }
}

struct Job {
    udid: String,
    asset: Asset,
    dest: PathBuf,
    settings: Settings,
    state: JobState,
    attempts: u32,
    overwrite: bool,
}

struct Done<M> {
    id: JobId,
    result: Result<bool, JobError>,
    conn: Option<(String, M)>,
}

struct Manager<O: Opener> {
    opener: Arc<O>,
    /// Destinations of unfinished jobs, so two items never get the same file
    /// (audit REL-01/02).
    reserved: HashMap<PathBuf, JobId>,
    settings: Settings,
    staging_dir: PathBuf,
    history: Option<History>,
    events: UnboundedSender<QueueEvent>,
    jobs: HashMap<JobId, Job>,
    pending: VecDeque<JobId>,
    running: HashSet<JobId>,
    conflicts: VecDeque<JobId>,
    shown_conflict: Option<JobId>,
    /// "Do this for all": answer for further conflicts until the queue idles.
    conflict_default: Option<bool>,
    /// Idle AFC connections, reused by the next job for the same phone.
    conns: Vec<(String, O::Media)>,
    user_paused: bool,
    device_paused: Option<String>,
    ready_udid: Option<String>,
    next_id: JobId,
}

impl<O: Opener> Manager<O> {
    async fn run(mut self, mut rx: UnboundedReceiver<QueueCommand>) {
        let (done_tx, mut done_rx) = mpsc::unbounded_channel::<Done<O::Media>>();
        loop {
            self.dispatch(&done_tx);
            self.show_conflict();
            if self.idle() {
                self.conflict_default = None;
            }
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => self.command(cmd),
                    None => return,
                },
                Some(done) = done_rx.recv() => self.finished(done),
            }
        }
    }

    fn emit(&self, ev: QueueEvent) {
        let _ = self.events.send(ev);
    }

    fn set_state(&mut self, id: JobId, state: JobState) {
        if let Some(j) = self.jobs.get_mut(&id) {
            j.state = state.clone();
            if state.is_finished() && self.reserved.get(&j.dest) == Some(&id) {
                self.reserved.remove(&j.dest);
            }
        }
        self.emit(QueueEvent::State { id, state });
    }

    fn idle(&self) -> bool {
        self.pending.is_empty() && self.running.is_empty() && self.conflicts.is_empty()
    }

    fn pause_reason(&self) -> Option<String> {
        self.device_paused
            .clone()
            .or_else(|| self.user_paused.then(|| "Paused".to_owned()))
    }

    fn command(&mut self, cmd: QueueCommand) {
        match cmd {
            QueueCommand::Add { udid, assets } => self.add(udid, assets),
            QueueCommand::SetSettings(s) => self.settings = s,
            QueueCommand::Resolve {
                id,
                overwrite,
                apply_to_all,
            } => {
                self.resolve(id, overwrite);
                if apply_to_all {
                    self.conflict_default = Some(overwrite);
                    while let Some(other) = self.conflicts.front().copied() {
                        self.resolve(other, overwrite);
                    }
                }
            }
            QueueCommand::Retry(id) => self.retry(id),
            QueueCommand::RetryAllFailed => {
                let failed: Vec<JobId> = self
                    .jobs
                    .iter()
                    .filter(|(_, j)| matches!(j.state, JobState::Failed { .. }))
                    .map(|(id, _)| *id)
                    .collect();
                let mut failed = failed;
                failed.sort_unstable();
                for id in failed {
                    self.retry(id);
                }
            }
            QueueCommand::Pause => {
                self.user_paused = true;
                self.emit(QueueEvent::Paused {
                    reason: self.pause_reason(),
                });
            }
            QueueCommand::Resume => {
                self.user_paused = false;
                self.emit(QueueEvent::Paused {
                    reason: self.pause_reason(),
                });
            }
            QueueCommand::DeviceReady(udid) => {
                self.ready_udid = Some(udid);
                if self.device_paused.take().is_some() {
                    self.emit(QueueEvent::Paused {
                        reason: self.pause_reason(),
                    });
                }
            }
            QueueCommand::DeviceGone => {
                self.ready_udid = None;
                self.conns.clear();
            }
            QueueCommand::ClearFinished => {
                let ids: Vec<JobId> = self
                    .jobs
                    .iter()
                    .filter(|(_, j)| {
                        matches!(
                            j.state,
                            JobState::Done | JobState::Overwritten | JobState::Skipped
                        )
                    })
                    .map(|(id, _)| *id)
                    .collect();
                for id in &ids {
                    self.jobs.remove(id);
                }
                self.emit(QueueEvent::Cleared { ids });
            }
        }
    }

    fn add(&mut self, udid: String, assets: Vec<Asset>) {
        let active: HashSet<String> = self
            .jobs
            .values()
            .filter(|j| !j.state.is_finished())
            .map(|j| j.asset.device_path())
            .collect();
        for asset in assets {
            if asset.trashed || active.contains(&asset.device_path()) {
                continue;
            }
            let id = self.next_id;
            self.next_id += 1;
            let reserved = &self.reserved;
            let dest = resolve_dest(&self.settings, &asset, |p| reserved.contains_key(p))
                .path()
                .to_path_buf();
            self.reserved.insert(dest.clone(), id);
            let name = dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| output_name(&asset));
            self.emit(QueueEvent::Added {
                id,
                name,
                source: asset.device_path(),
                dest: dest.clone(),
            });
            self.jobs.insert(
                id,
                Job {
                    udid: udid.clone(),
                    asset,
                    dest,
                    settings: self.settings.clone(),
                    state: JobState::Queued,
                    attempts: 0,
                    overwrite: false,
                },
            );
            self.emit(QueueEvent::State {
                id,
                state: JobState::Queued,
            });
            self.pending.push_back(id);
        }
    }

    fn resolve(&mut self, id: JobId, overwrite: bool) {
        self.conflicts.retain(|c| *c != id);
        if self.shown_conflict == Some(id) {
            self.shown_conflict = None;
        }
        if overwrite {
            if let Some(j) = self.jobs.get_mut(&id) {
                j.overwrite = true;
            }
            self.set_state(id, JobState::Queued);
            self.pending.push_front(id);
        } else {
            self.set_state(id, JobState::Skipped);
        }
    }

    fn retry(&mut self, id: JobId) {
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        if matches!(j.state, JobState::Failed { .. }) {
            j.attempts = 0;
            self.set_state(id, JobState::Queued);
            self.pending.push_back(id);
        }
    }

    /// Starts jobs while there is a free worker and nothing is paused.
    fn dispatch(&mut self, done_tx: &UnboundedSender<Done<O::Media>>) {
        while self.running.len() < WORKERS && self.pause_reason().is_none() {
            let Some(id) = self.pending.pop_front() else {
                break;
            };
            let Some(job) = self.jobs.get_mut(&id) else {
                continue;
            };
            if self.ready_udid.as_deref() != Some(job.udid.as_str()) {
                self.pending.push_front(id);
                self.device_paused = Some("Connect and unlock your iPhone to continue".into());
                self.emit(QueueEvent::Paused {
                    reason: self.pause_reason(),
                });
                break;
            }

            // Look again: files may have come or gone since the job was added.
            // Only this item's own earlier copy is a conflict (PLAN §7.5);
            // anything else there makes it pick the next free name.
            let reserved = &self.reserved;
            let target = resolve_dest(&job.settings, &job.asset, |p| {
                reserved.get(p).is_some_and(|other| *other != id)
            });
            if target.path() != job.dest {
                if self.reserved.get(&job.dest) == Some(&id) {
                    self.reserved.remove(&job.dest);
                }
                job.dest = target.path().to_path_buf();
                let _ = self.events.send(QueueEvent::Retarget {
                    id,
                    dest: job.dest.clone(),
                });
            }
            self.reserved.insert(job.dest.clone(), id);
            if matches!(target, Target::Free(_)) {
                job.overwrite = false;
            } else if !job.overwrite {
                let policy = match (self.conflict_default, job.settings.on_conflict) {
                    (Some(true), _) | (None, ConflictPolicy::Overwrite) => {
                        ConflictPolicy::Overwrite
                    }
                    (Some(false), _) | (None, ConflictPolicy::Skip) => ConflictPolicy::Skip,
                    (None, ConflictPolicy::Ask) => ConflictPolicy::Ask,
                };
                match policy {
                    ConflictPolicy::Overwrite => job.overwrite = true,
                    ConflictPolicy::Skip => {
                        self.set_state(id, JobState::Skipped);
                        continue;
                    }
                    ConflictPolicy::Ask => {
                        self.conflicts.push_back(id);
                        self.set_state(id, JobState::Conflict);
                        continue;
                    }
                }
            }

            let spec = JobSpec {
                id,
                asset: job.asset.clone(),
                dest: job.dest.clone(),
                overwrite: job.overwrite,
                settings: job.settings.clone(),
                staging_dir: self.staging_dir.clone(),
            };
            let udid = job.udid.clone();
            let conn = self
                .conns
                .iter()
                .position(|(u, _)| *u == udid)
                .map(|i| self.conns.swap_remove(i).1);
            self.running.insert(id);
            let (events, done_tx) = (self.events.clone(), done_tx.clone());
            let opener = self.opener.clone();
            tokio::spawn(async move {
                let mut fs = match conn {
                    Some(fs) => fs,
                    None => match opener.open(&udid).await {
                        Ok(fs) => fs,
                        Err(e) => {
                            let result = Err(if e.is_connection_problem() {
                                JobError::Connection(e.to_string())
                            } else {
                                JobError::Failed {
                                    stage: Stage::Read,
                                    error: e.to_string(),
                                }
                            });
                            let _ = done_tx.send(Done {
                                id,
                                result,
                                conn: None,
                            });
                            return;
                        }
                    },
                };
                let result = job::run(&spec, &mut fs, &events).await;
                // Keep the connection only after a clean run.
                let conn = result.is_ok().then_some((udid, fs));
                let _ = done_tx.send(Done { id, result, conn });
            });
        }
    }

    fn finished(&mut self, done: Done<O::Media>) {
        self.running.remove(&done.id);
        if let Some(conn) = done.conn {
            self.conns.push(conn);
        }
        let id = done.id;
        match done.result {
            Ok(replaced) => self.set_state(
                id,
                if replaced {
                    JobState::Overwritten
                } else {
                    JobState::Done
                },
            ),
            Err(JobError::Connection(msg)) => {
                // Back to the front of the queue; resume when the phone is back.
                self.conns.clear();
                self.set_state(id, JobState::Queued);
                self.pending.push_front(id);
                if self.device_paused.is_none() {
                    self.device_paused = Some(format!(
                        "Reconnect and unlock your iPhone to continue ({msg})"
                    ));
                    self.emit(QueueEvent::Paused {
                        reason: self.pause_reason(),
                    });
                }
            }
            Err(JobError::Failed { stage, error }) => {
                let Some(job) = self.jobs.get_mut(&id) else {
                    return;
                };
                job.attempts += 1;
                if job.attempts <= AUTO_RETRIES {
                    self.set_state(id, JobState::Queued);
                    self.pending.push_back(id);
                    return;
                }
                let record = FailureRecord {
                    time: chrono::Local::now().to_rfc3339(),
                    udid: job.udid.clone(),
                    source: job.asset.save_plan().source,
                    dest: job.dest.display().to_string(),
                    stage: stage.as_str().to_owned(),
                    error: error.clone(),
                    retries: job.attempts - 1,
                };
                if let Some(h) = &self.history {
                    let _ = h.record(&record);
                }
                self.set_state(id, JobState::Failed { stage, error });
            }
        }
    }

    /// Emits the first waiting conflict once, so the UI shows one dialog at a time.
    fn show_conflict(&mut self) {
        let front = self.conflicts.front().copied();
        if front == self.shown_conflict {
            return;
        }
        self.shown_conflict = front;
        let Some(id) = front else {
            self.emit(QueueEvent::ConflictCleared);
            return;
        };
        let Some(job) = self.jobs.get(&id) else {
            return;
        };
        let meta = std::fs::metadata(&job.dest).ok();
        let others = self.conflicts.len() - 1
            + self
                .pending
                .iter()
                .filter_map(|p| self.jobs.get(p))
                .filter(|j| {
                    !j.overwrite
                        && std::fs::metadata(&j.dest)
                            .and_then(|m| m.modified())
                            .is_ok_and(|t| crate::layout::is_copy_of(t, &j.asset))
                })
                .count();
        self.emit(QueueEvent::Conflict {
            id,
            name: output_name(&job.asset),
            dest: job.dest.clone(),
            existing_size: meta.as_ref().map_or(0, |m| m.len()),
            existing_modified: meta.and_then(|m| m.modified().ok()),
            new_size: job.asset.original_size,
            others,
        });
    }
}
