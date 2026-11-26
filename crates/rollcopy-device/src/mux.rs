//! Where usbmuxd is. Normally its own socket, `/var/run/usbmuxd`. Inside
//! Flatpak the app can't keep a working view of that socket (it's replaced
//! on every re-plug, and the sandbox only sees the one that existed when it
//! started), so it talks to a relay instead ([`crate::relay`]); the app
//! points this crate at it once, at startup.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use idevice::IdeviceError;
use idevice::usbmuxd::{UsbmuxdAddr, UsbmuxdConnection};

static SOCKET: OnceLock<PathBuf> = OnceLock::new();

/// Uses the usbmuxd socket at `path` instead of the system one, for the
/// rest of the process. Only the first call counts; returns whether it was
/// this one.
pub fn use_socket(path: PathBuf) -> bool {
    SOCKET.set(path).is_ok()
}

/// A socket other than the system one is in use (Flatpak).
pub fn relayed() -> bool {
    SOCKET.get().is_some()
}

/// The socket every usbmuxd connection goes to.
pub(crate) fn addr() -> UsbmuxdAddr {
    match SOCKET.get() {
        Some(p) => UsbmuxdAddr::UnixSocket(p.to_string_lossy().into_owned()),
        None => UsbmuxdAddr::default(),
    }
}

/// A new connection to usbmuxd.
pub(crate) async fn connect() -> Result<UsbmuxdConnection, IdeviceError> {
    addr().connect(0).await
}

/// The system's usbmuxd socket, as Flatpak's `--filesystem=/run/usbmuxd`
/// shows it (`/var/run` is a link to `/run`).
pub const SYSTEM_SOCKET: &str = "/run/usbmuxd";

/// Whether something answers at the usbmuxd socket `path`.
pub fn socket_answers(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}
