//! Background thread that owns all device I/O (PLAN §4: never on the UI
//! thread). Runs the connection-guide watcher on a tokio runtime, loads the
//! library whenever a phone becomes ready, and serves thumbnail and "open"
//! requests from the UI. Everything is read-only towards the phone.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};

use rollcopy_device::{DeviceError, MediaFs, Phase, Session};
use rollcopy_library::{Asset, Library};
use rollcopy_queue::{History, QueueCommand, QueueEvent, QueueHandle, Settings};

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
    /// A full-size image for the viewer is ready (or failed). `key` is the
    /// asset's device path.
    FullImage {
        key: String,
        result: Result<PathBuf, String>,
    },
    /// Whether a tray host accepted the icon.
    TrayAvailable(bool),
    Tray(crate::tray::TrayAction),
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
    /// Fetch a full-size photo for the viewer into `dest` (HEIC → JPG).
    FullImage {
        udid: String,
        key: String,
        source: String,
        dest: PathBuf,
    },
}

#[derive(Debug, Clone)]
pub struct ThumbReq {
    pub key: String,
    /// Phone path of iOS's own thumbnail (`.../5005.JPG`).
    pub source: String,
    pub dest: PathBuf,
}

/// App directories: cache root (`~/.cache/rollcopy`) and state directory
/// (`~/.local/state/rollcopy`, for the failure history).
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

    /// Local thumbnail for an asset: `thumbs/<udid>/<dir>/<file>.jpg`, or
    /// `<file>.e<edit time>.jpg` for edited items, so editing a photo on the
    /// phone after it was cached gives it a fresh thumbnail.
    pub fn thumb(&self, udid: &str, a: &Asset) -> PathBuf {
        let name = if a.is_edited() {
            format!("{}.e{}.jpg", a.filename, a.edited_at.unwrap_or(0))
        } else {
            format!("{}.jpg", a.filename)
        };
        self.root
            .join("thumbs")
            .join(udid)
            .join(&a.directory)
            .join(name)
    }

    /// Full-size image for the viewer: `view/<udid>/<dir>/<file>[.e<edit>].<ext>`,
    /// with HEIC stored as JPG. Cleared at startup.
    pub fn view(&self, udid: &str, a: &Asset) -> PathBuf {
        let plan = a.save_plan();
        let ext = match plan.from_ext.as_str() {
            "HEIC" | "HEIF" => "jpg".to_owned(),
            other => other.to_ascii_lowercase(),
        };
        let name = match (a.is_edited(), a.edited_at) {
            (true, t) => format!("{}.e{}.{ext}", a.stem(), t.unwrap_or(0)),
            (false, _) => format!("{}.{ext}", a.stem()),
        };
        self.root
            .join("view")
            .join(udid)
            .join(&a.directory)
            .join(name)
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
    /// Tray status updates from the UI.
    pub tray: async_channel::Sender<crate::tray::TrayState>,
}

impl CachePaths {
    /// Makes the cache and state folders private, and clears what is only
    /// kept for one session: viewer images, opened files, and a database
    /// copy or staging files left by a crash (audit SEC-02, REL-09). Called
    /// at startup, under the single-instance lock, and at quit.
    pub fn tidy(&self) {
        let _ = crate::config::create_private_dir(&self.root);
        let _ = crate::config::create_private_dir(&self.state);
        for dir in [
            self.root.join("view"),
            self.open_dir(),
            self.staging(),
            self.root.join("db"),
        ] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Starts the worker thread.
pub fn spawn(paths: CachePaths) -> Worker {
    paths.tidy();
    let (ev_tx, ev_rx) = async_channel::unbounded();
    let (cmd_tx, cmd_rx) = async_channel::unbounded();
    let (tray_tx, tray_rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("rollcopy-device".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(async move {
                crate::notify::start();
                if std::env::var_os("ROLLCOPY_DEBUG_NOTIFY").is_some() {
                    crate::notify::send(
                        "Rollcopy notification test",
                        "If you can read this, notifications work.",
                    );
                }
                tokio::spawn(crate::instance::serve(ev_tx.clone()));
                start_tray(tray_rx, ev_tx.clone());
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
        tray: tray_tx,
    }
}

/// Starts the tray icon and forwards its menu actions to the UI.
fn start_tray(
    updates: async_channel::Receiver<crate::tray::TrayState>,
    tx: async_channel::Sender<Event>,
) {
    let (act_tx, act_rx) = async_channel::unbounded();
    let tx2 = tx.clone();
    tokio::spawn(async move {
        let ok = crate::tray::run(updates, act_tx).await;
        let _ = tx2.send(Event::TrayAvailable(ok)).await;
    });
    tokio::spawn(async move {
        while let Ok(a) = act_rx.recv().await {
            if tx.send(Event::Tray(a)).await.is_err() {
                break;
            }
        }
    });
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
    let handle = rollcopy_queue::spawn(
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
    rollcopy_device::watch(move |phase| {
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
    rollcopy_library::load(&session, &dir)
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
                Command::FullImage {
                    udid,
                    key,
                    source,
                    dest,
                } => {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let result = fetch_full(&udid, &source, &dest)
                            .await
                            .map(|()| dest)
                            .map_err(|e| e.to_string());
                        let _ = tx.send(Event::FullImage { key, result }).await;
                    });
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

/// Copies a photo from the phone for the viewer, converting HEIC to JPG.
async fn fetch_full(udid: &str, source: &str, dest: &Path) -> Result<(), DeviceError> {
    if tokio::fs::try_exists(dest).await.unwrap_or(false) {
        return Ok(());
    }
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let mut conn = None;
    connect(&mut conn, udid).await?;
    let fs = &mut conn.as_mut().expect("connected").fs;
    let is_heic = source.to_ascii_lowercase().ends_with(".heic")
        || source.to_ascii_lowercase().ends_with(".heif");
    let part = part_path(dest);
    if is_heic {
        let raw = part_path(&part);
        fs.copy_to_local(source, &raw, |_| {}).await?;
        let (src, out) = (raw.clone(), part.clone());
        let converted = tokio::task::spawn_blocking(move || {
            rollcopy_convert::heic_to_jpeg(&src, &out, 90).map(|_| ())
        })
        .await;
        let _ = tokio::fs::remove_file(&raw).await;
        match converted {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(DeviceError::Other(e.to_string())),
            Err(e) => return Err(DeviceError::Other(e.to_string())),
        }
    } else {
        fs.copy_to_local(source, &part, |_| {}).await?;
    }
    tokio::fs::rename(&part, dest).await?;
    Ok(())
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
