//! Running as a Flatpak (PLAN §15, Flatpak).
//!
//! The sandbox can't keep a working view of the host's usbmuxd socket, so
//! Rollcopy talks to the phone through a relay ([`rollcopy_device::relay`])
//! in a socket in its runtime folder. A background thread keeps one
//! running: whenever the relay doesn't answer, it asks the Flatpak portal
//! (`flatpak-spawn`, never `--host`) for a fresh sandbox running
//! `rollcopy --usbmuxd-relay`, which sees the usbmuxd socket as it is now.
//! Outside Flatpak none of this runs.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rollcopy_device::mux;
use rollcopy_device::relay;

/// Command-line flag of the relay helper.
const RELAY_FLAG: &str = "--usbmuxd-relay";
/// How often the app checks the relay while it's down.
const CHECK_EVERY: Duration = Duration::from_secs(3);

/// Whether Rollcopy runs inside Flatpak.
pub fn active() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// The app's runtime folder, shared by all of its sandboxes:
/// `$XDG_RUNTIME_DIR/app/$FLATPAK_ID`.
fn shared_dir() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let id = std::env::var_os("FLATPAK_ID")?;
    Some(PathBuf::from(runtime).join("app").join(id))
}

struct Paths {
    relay: PathBuf,
    owner: PathBuf,
}

fn paths(dir: &Path) -> Paths {
    Paths {
        relay: dir.join("usbmuxd"),
        owner: dir.join("relay.owner"),
    }
}

/// If this process was started as the relay helper, serves the relay and
/// returns `true` once it ends; the caller then exits. Quiet: it has no
/// window, and stops by itself.
pub fn run_relay_if_asked() -> bool {
    if !std::env::args().any(|a| a == RELAY_FLAG) {
        return false;
    }
    let Some(dir) = shared_dir() else {
        return true;
    };
    let p = paths(&dir);
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return true;
    };
    let end = rt.block_on(relay::serve(
        &p.relay,
        Path::new(mux::SYSTEM_SOCKET),
        &p.owner,
    ));
    if let Err(e) = end {
        eprintln!("usbmuxd relay: {e}");
    }
    true
}

/// Keeps a relay running while the app runs. Dropping it releases the
/// owner lock, and the relay stops within a couple of seconds.
pub struct RelayKeeper {
    _owner: File,
}

/// In Flatpak: points all phone access at the relay and starts the thread
/// that keeps one running. `None` outside Flatpak, or if the runtime
/// folder can't be used (then phone access fails as it would without it).
pub fn start_relay() -> Option<RelayKeeper> {
    if !active() {
        return None;
    }
    let dir = shared_dir()?;
    if let Err(e) = std::fs::create_dir_all(&dir) {
        crate::applog!("flatpak: no runtime folder for the usbmuxd relay: {e}");
        return None;
    }
    let p = paths(&dir);
    let owner = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&p.owner)
        .ok()?;
    // Held for the app's life; the relay stops when it's released.
    if owner.try_lock().is_err() {
        crate::applog!("flatpak: another Rollcopy owns the usbmuxd relay");
    }
    mux::use_socket(p.relay.clone());
    let relay = p.relay;
    let spawned = std::thread::Builder::new()
        .name("usbmuxd-relay".into())
        .spawn(move || keep_relay(&relay));
    if let Err(e) = spawned {
        crate::applog!("flatpak: couldn't start the usbmuxd relay thread: {e}");
    }
    crate::applog!("flatpak: phone access goes through the usbmuxd relay");
    Some(RelayKeeper { _owner: owner })
}

/// Starts a relay helper whenever the relay doesn't answer and none is
/// running. With no phone plugged in, each helper finds no usbmuxd and
/// exits at once, so this tries again every few seconds; a phone shows up
/// within that time of being plugged in.
fn keep_relay(relay: &Path) {
    let Ok(exe) = std::env::current_exe() else {
        crate::applog!("flatpak: can't find the app's own binary for the usbmuxd relay");
        return;
    };
    let mut helper: Option<Child> = None;
    let mut warned = false;
    loop {
        let running = helper
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if !running && !mux::socket_answers(relay) {
            match Command::new("flatpak-spawn")
                .arg("--watch-bus")
                .arg(&exe)
                .arg(RELAY_FLAG)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .spawn()
            {
                Ok(c) => helper = Some(c),
                Err(e) if !warned => {
                    warned = true;
                    crate::applog!("flatpak: couldn't start the usbmuxd relay: {e}");
                }
                Err(_) => {}
            }
        }
        std::thread::sleep(CHECK_EVERY);
    }
}
