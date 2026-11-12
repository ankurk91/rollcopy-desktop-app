//! Background thread that owns all device I/O (PLAN §4: never on the UI
//! thread). Runs the connection-guide watcher on a tokio runtime and loads the
//! library whenever a phone becomes ready. Results go to the UI over a channel.

use std::path::PathBuf;

use unspool_device::{Phase, Session};
use unspool_library::Library;

#[derive(Debug, Clone)]
pub enum Event {
    Phase(Phase),
    LibraryLoading,
    Library(Result<Library, String>),
}

/// Starts the worker thread. `cache_dir` is the app cache root
/// (`~/.cache/unspool`); database copies go under `db/<udid>/`.
pub fn spawn(cache_dir: PathBuf) -> async_channel::Receiver<Event> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("unspool-device".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run(tx, cache_dir));
        })
        .expect("spawn device thread");
    rx
}

async fn run(tx: async_channel::Sender<Event>, cache_dir: PathBuf) {
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
        // Phase first, so the UI never sees library events for a phone it
        // doesn't know is ready. The UI may have quit; nothing to do then.
        let _ = tx.try_send(Event::Phase(phase));
        if let Some(udid) = ready_udid {
            let dir = cache_dir.join("db").join(&udid);
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
