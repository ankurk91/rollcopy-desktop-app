//! Background thread that owns all device I/O (PLAN §4: never on the UI
//! thread). Runs the connection-guide watcher on a tokio runtime, loads the
//! library whenever a phone becomes ready, and serves thumbnail and "open"
//! requests from the UI. Everything is read-only towards the phone.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use unspool_device::{DeviceError, MediaFs, Phase, Session};
use unspool_library::{Asset, Library};
use unspool_queue::{History, QueueCommand, QueueEvent, QueueHandle, Settings};

#[derive(Debug, Clone)]
pub enum Event {
    Phase(Phase),
    LibraryLoading,
    Library(Result<Library, String>),
    /// A thumbnail is now in the cache. `key` is the asset's device path.
    Thumb {
        key: String,
        path: PathBuf,
    },
    /// The phone has no thumbnail for this item.
    ThumbMissing {
        key: String,
    },
    /// "Open" finished copying and handed the file to the system viewer.
    Opened {
        name: String,
    },
    OpenFailed {
        name: String,
        error: String,
    },
    Queue(QueueEvent),
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Fetch thumbnails; the newest request is served first.
    Thumbs {
        udid: String,
        items: Vec<ThumbReq>,
    },
    /// Copy one file to a temp folder and open it with `xdg-open`.
    Open {
        udid: String,
        device_path: String,
        name: String,
    },
    Queue(QueueCommand),
}

#[derive(Debug, Clone)]
pub struct ThumbReq {
    pub key: String,
    /// Phone path of iOS's own thumbnail (`.../5005.JPG`).
    pub source: String,
    pub dest: PathBuf,
}

/// App directories: cache root (`~/.cache/unspool`) and state directory
/// (`~/.local/state/unspool`, for the failure history).
#[derive(Debug, Clone)]
pub struct CachePaths {
    pub root: PathBuf,
    pub state: PathBuf,
}

impl CachePaths {
    pub fn staging(&self) -> PathBuf {
        self.root.join("staging")
    }

    pub fn history(&self) -> PathBuf {
        self.state.join("history.sqlite")
    }

    pub fn db(&self, udid: &str) -> PathBuf {
        self.root.join("db").join(udid)
    }

    /// Local thumbnail for an asset: `thumbs/<udid>/<dir>/<file>.jpg`.
    pub fn thumb(&self, udid: &str, a: &Asset) -> PathBuf {
        self.root
            .join("thumbs")
            .join(udid)
            .join(&a.directory)
            .join(format!("{}.jpg", a.filename))
    }

    pub fn open_dir(&self) -> PathBuf {
        self.root.join("open")
    }
}

/// Phone path of iOS's thumbnail for an asset. Present for every item on
/// iOS 27 (checked 2026-11-12: every item had one); 480×360 for photos, 640×360 for videos.
pub fn phone_thumb_path(a: &Asset) -> String {
    format!(
        "/PhotoData/Thumbnails/V2/{}/{}/5005.JPG",
        a.directory, a.filename
    )
}

pub struct Worker {
    pub events: async_channel::Receiver<Event>,
    pub commands: async_channel::Sender<Command>,
}

/// Starts the worker thread.
pub fn spawn(paths: CachePaths) -> Worker {
    let (ev_tx, ev_rx) = async_channel::unbounded();
    let (cmd_tx, cmd_rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("unspool-device".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                let queue = start_queue(&paths, ev_tx.clone());
                tokio::spawn(serve_commands(
                    cmd_rx,
                    ev_tx.clone(),
                    paths.clone(),
                    queue.clone(),
                ));
                run_watch(ev_tx, paths, queue).await;
            });
        })
        .expect("spawn device thread");
    Worker {
        events: ev_rx,
        commands: cmd_tx,
    }
}

/// Starts the transfer queue and forwards its events to the UI. The UI sends
/// the real destination with `SetSettings` once it knows the phone's name.
fn start_queue(paths: &CachePaths, tx: async_channel::Sender<Event>) -> QueueHandle {
    let (qtx, mut qrx) = tokio::sync::mpsc::unbounded_channel();
    let history = match History::open(&paths.history()) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("history disabled: {e}");
            None
        }
    };
    let handle = unspool_queue::spawn(
        Settings::new(paths.root.join("unset-destination")),
        paths.staging(),
        history,
        qtx,
    );
    tokio::spawn(async move {
        while let Some(ev) = qrx.recv().await {
            if tx.send(Event::Queue(ev)).await.is_err() {
                break;
            }
        }
    });
    handle
}

async fn run_watch(tx: async_channel::Sender<Event>, paths: CachePaths, queue: QueueHandle) {
    let mut loading: Option<tokio::task::JoinHandle<()>> = None;
    unspool_device::watch(move |phase| {
        // Any change of phase invalidates a library load in progress.
        if let Some(task) = loading.take() {
            task.abort();
        }
        let ready_udid = match &phase {
            Phase::Ready { info } => Some(info.udid.clone()),
            _ => None,
        };
        // The queue pauses on its own when a read fails; this tells it when
        // it may continue.
        queue.send(match &ready_udid {
            Some(u) => QueueCommand::DeviceReady(u.clone()),
            None => QueueCommand::DeviceGone,
        });
        // Phase first, so the UI never sees library events for a phone it
        // doesn't know is ready. The UI may have quit; nothing to do then.
        let _ = tx.try_send(Event::Phase(phase));
        if let Some(udid) = ready_udid {
            let dir = paths.db(&udid);
            let tx = tx.clone();
            loading = Some(tokio::spawn(async move {
                let _ = tx.send(Event::LibraryLoading).await;
                let result = load_library(&udid, dir).await;
                let _ = tx.send(Event::Library(result)).await;
            }));
        }
    })
    .await;
}

async fn load_library(udid: &str, dir: PathBuf) -> Result<Library, String> {
    let session = Session::open(udid).await.map_err(|e| e.to_string())?;
    unspool_library::load(&session, &dir)
        .await
        .map_err(|e| e.to_string())
}

/// One open AFC connection, reopened after errors or a device change.
struct Conn {
    udid: String,
    fs: MediaFs,
}

async fn connect(conn: &mut Option<Conn>, udid: &str) -> Result<(), DeviceError> {
    if conn.as_ref().is_some_and(|c| c.udid == udid) {
        return Ok(());
    }
    let session = Session::open(udid).await?;
    let fs = session.media().await?;
    *conn = Some(Conn {
        udid: udid.to_owned(),
        fs,
    });
    Ok(())
}

async fn serve_commands(
    rx: async_channel::Receiver<Command>,
    tx: async_channel::Sender<Event>,
    paths: CachePaths,
    transfers: QueueHandle,
) {
    // Thumbnail queue, newest first; `queued` de-duplicates keys.
    let mut queue: VecDeque<(String, ThumbReq)> = VecDeque::new();
    let mut queued: HashSet<String> = HashSet::new();
    let mut conn: Option<Conn> = None;

    loop {
        // Take everything that arrived; block only when idle.
        let mut incoming = Vec::new();
        if queue.is_empty() {
            match rx.recv().await {
                Ok(c) => incoming.push(c),
                Err(_) => return,
            }
        }
        while let Ok(c) = rx.try_recv() {
            incoming.push(c);
        }

        for cmd in incoming {
            match cmd {
                Command::Queue(cmd) => transfers.send(cmd),
                Command::Thumbs { udid, items } => {
                    for item in items.into_iter().rev() {
                        if queued.insert(item.key.clone()) {
                            queue.push_front((udid.clone(), item));
                        }
                    }
                }
                Command::Open {
                    udid,
                    device_path,
                    name,
                } => {
                    // Own task and connection: a large video must not hold
                    // up thumbnails while it copies.
                    let (tx, paths) = (tx.clone(), paths.clone());
                    tokio::spawn(async move {
                        let mut own = None;
                        let ev = match open_file(&mut own, &udid, &device_path, &name, &paths).await
                        {
                            Ok(()) => Event::Opened { name },
                            Err(e) => Event::OpenFailed {
                                name,
                                error: e.to_string(),
                            },
                        };
                        let _ = tx.send(ev).await;
                    });
                }
            }
        }

        let Some((udid, req)) = queue.pop_front() else {
            continue;
        };
        queued.remove(&req.key);
        let ev = match fetch_thumb(&mut conn, &udid, &req).await {
            Ok(()) => Event::Thumb {
                key: req.key,
                path: req.dest,
            },
            Err(DeviceError::NotFound(_)) => Event::ThumbMissing { key: req.key },
            Err(e) if e.is_connection_problem() => {
                // Phone gone or locked out: drop the queue; the UI asks again
                // for whatever is on screen once the phone is back.
                conn = None;
                queue.clear();
                queued.clear();
                continue;
            }
            Err(_) => {
                conn = None;
                Event::ThumbMissing { key: req.key }
            }
        };
        let _ = tx.send(ev).await;
    }
}

async fn fetch_thumb(
    conn: &mut Option<Conn>,
    udid: &str,
    req: &ThumbReq,
) -> Result<(), DeviceError> {
    if req.dest.exists() {
        return Ok(());
    }
    connect(conn, udid).await?;
    let fs = &mut conn.as_mut().expect("connected").fs;
    if let Some(dir) = req.dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let part = part_path(&req.dest);
    fs.copy_to_local(&req.source, &part, |_| {}).await?;
    tokio::fs::rename(&part, &req.dest).await?;
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

async fn open_file(
    conn: &mut Option<Conn>,
    udid: &str,
    device_path: &str,
    name: &str,
    paths: &CachePaths,
) -> Result<(), DeviceError> {
    let dir = paths.open_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let dest = dir.join(name);
    connect(conn, udid).await?;
    let fs = &mut conn.as_mut().expect("connected").fs;
    let size = fs.stat(device_path).await?.size;
    let have = tokio::fs::metadata(&dest).await.map(|m| m.len()).ok();
    if have != Some(size) {
        let part = part_path(&dest);
        fs.copy_to_local(device_path, &part, |_| {}).await?;
        tokio::fs::rename(&part, &dest).await?;
    }
    let status = tokio::process::Command::new("xdg-open")
        .arg(&dest)
        .status()
        .await?;
    if !status.success() {
        return Err(DeviceError::Other(format!("xdg-open exited with {status}")));
    }
    Ok(())
}
