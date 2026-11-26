//! The usbmuxd relay used inside Flatpak (PLAN §15, Flatpak).
//!
//! A Flatpak sandbox only sees the usbmuxd socket that existed when it
//! started, and usbmuxd makes a new one each time a phone is plugged in. So
//! the app asks the Flatpak portal for a fresh sandbox running
//! `rollcopy --usbmuxd-relay`, which serves [`serve`]: it listens on a
//! socket in the app's runtime folder (shared by all of the app's
//! sandboxes) and passes every connection through to the usbmuxd it sees,
//! byte for byte. When that socket goes stale (re-plug) or the app is gone,
//! it stops; the app starts a new one when it needs the phone again.
//! Tested by hand on 2026-11-26 with a phone unplugged and re-plugged.
//!
//! It only forwards bytes; it adds no way to change the phone (PLAN §2a).

use std::fs::{File, TryLockError};
use std::os::unix::fs::MetadataExt as _;
use std::path::Path;
use std::time::Duration;

use tokio::net::{UnixListener, UnixStream};

/// How often the relay checks that usbmuxd and the app are still there.
const CHECK_EVERY: Duration = Duration::from_secs(2);

/// Why [`serve`] returned.
#[derive(Debug, PartialEq, Eq)]
pub enum RelayEnd {
    /// Nothing answers at the usbmuxd socket (no phone plugged in yet).
    NoUsbmuxd,
    /// The usbmuxd socket stopped answering (the phone was re-plugged).
    UsbmuxdGone,
    /// The app holding `owner_lock` has exited.
    OwnerGone,
}

/// Whether the process holding a lock on `lock` is still running. The app
/// keeps that lock while it runs; the kernel drops it when the app exits,
/// even after a crash.
pub fn owner_alive(lock: &Path) -> bool {
    let Ok(f) = File::options().read(true).write(true).open(lock) else {
        return false;
    };
    match f.try_lock() {
        // We got it, so nobody holds it; it's released as `f` drops.
        Ok(()) => false,
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => false,
    }
}

/// Relays connections from `listen` to the usbmuxd socket `usbmuxd` until
/// that stops answering or the app holding `owner_lock` exits. Replaces a
/// stale socket at `listen`; removes its own when done.
pub async fn serve(listen: &Path, usbmuxd: &Path, owner_lock: &Path) -> std::io::Result<RelayEnd> {
    if !crate::mux::socket_answers(usbmuxd) {
        return Ok(RelayEnd::NoUsbmuxd);
    }
    match std::fs::remove_file(listen) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let listener = UnixListener::bind(listen)?;
    let ours = std::fs::metadata(listen)?.ino();
    let mut tick = tokio::time::interval(CHECK_EVERY);
    let end = loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((client, _)) = accepted else { continue };
                let usbmuxd = usbmuxd.to_path_buf();
                tokio::spawn(async move {
                    if let Ok(mut upstream) = UnixStream::connect(&usbmuxd).await {
                        let mut client = client;
                        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                    }
                });
            }
            _ = tick.tick() => {
                if !owner_alive(owner_lock) {
                    break RelayEnd::OwnerGone;
                }
                if !crate::mux::socket_answers(usbmuxd) {
                    break RelayEnd::UsbmuxdGone;
                }
            }
        }
    };
    drop(listener);
    // Only our own socket: a newer relay may already have replaced it.
    if std::fs::metadata(listen).is_ok_and(|m| m.ino() == ours) {
        let _ = std::fs::remove_file(listen);
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A stand-in usbmuxd that echoes what it gets.
    fn echo_server(path: &Path) -> tokio::task::JoinHandle<()> {
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 64];
                    while let Ok(n) = s.read(&mut buf).await {
                        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                });
            }
        })
    }

    async fn round_trip(path: &Path) -> Vec<u8> {
        let mut c = UnixStream::connect(path).await.unwrap();
        c.write_all(b"hello").await.unwrap();
        let mut buf = [0u8; 5];
        c.read_exact(&mut buf).await.unwrap();
        buf.to_vec()
    }

    #[tokio::test]
    async fn relays_until_usbmuxd_goes_away() {
        let dir = tempfile::tempdir().unwrap();
        let (mux, relay, lock) = (
            dir.path().join("usbmuxd"),
            dir.path().join("relay"),
            dir.path().join("owner"),
        );
        let owner = File::create(&lock).unwrap();
        owner.lock().unwrap();

        // No usbmuxd yet: returns at once, no socket left behind.
        assert_eq!(
            serve(&relay, &mux, &lock).await.unwrap(),
            RelayEnd::NoUsbmuxd
        );
        assert!(!relay.exists());

        let server = echo_server(&mux);
        let r = relay.clone();
        let (m, l) = (mux.clone(), lock.clone());
        let task = tokio::spawn(async move { serve(&r, &m, &l).await });
        while !relay.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(round_trip(&relay).await, b"hello");

        // Re-plug: usbmuxd's socket is replaced; the relay notices and ends.
        server.abort();
        std::fs::remove_file(&mux).unwrap();
        let end = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(end, RelayEnd::UsbmuxdGone);
        assert!(!relay.exists());
        drop(owner);
    }

    #[tokio::test]
    async fn ends_when_the_app_exits() {
        let dir = tempfile::tempdir().unwrap();
        let (mux, relay, lock) = (
            dir.path().join("usbmuxd"),
            dir.path().join("relay"),
            dir.path().join("owner"),
        );
        let _server = echo_server(&mux);
        let owner = File::create(&lock).unwrap();
        owner.lock().unwrap();
        assert!(owner_alive(&lock));
        let (r, m, l) = (relay.clone(), mux.clone(), lock.clone());
        let task = tokio::spawn(async move { serve(&r, &m, &l).await });
        while !relay.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(owner);
        assert!(!owner_alive(&lock));
        let end = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(end, RelayEnd::OwnerGone);
    }
}
