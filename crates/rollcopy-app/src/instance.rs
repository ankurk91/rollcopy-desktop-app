//! One Rollcopy per user session (audit REL-03).
//!
//! Closing the window during a copy keeps Rollcopy in the tray, so starting
//! it again from the menu is the normal way back. A second process would
//! share the staging, viewer and database cache files with the first one,
//! so the second launch asks the first to show its window and exits.
//!
//! The lock is a `flock` on `<state dir>/rollcopy.lock`, released by the OS
//! when the process ends, even after a crash. The first instance also owns
//! the D-Bus name [`crate::APP_ID`] and answers
//! `org.freedesktop.Application.Activate`.

use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::path::Path;
use std::time::Duration;

use zbus::zvariant::{OwnedValue, Value};

use crate::APP_ID;
use crate::tray::TrayAction;
use crate::worker::Event;

const OBJECT_PATH: &str = "/io/github/ankurk91/Rollcopy";
const IFACE: &str = "org.freedesktop.Application";

pub enum Lock {
    /// This is the only instance; keep the file open until exit.
    Acquired(File),
    /// Another Rollcopy holds the lock.
    Held,
    /// No lock could be taken (unwritable state dir); run anyway.
    Unavailable,
}

/// Takes the instance lock in `dir`.
pub fn lock(dir: &Path) -> Lock {
    if crate::config::create_private_dir(dir).is_err() {
        return Lock::Unavailable;
    }
    let file = match File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("rollcopy.lock"))
    {
        Ok(f) => f,
        Err(_) => return Lock::Unavailable,
    };
    match file.try_lock() {
        Ok(()) => Lock::Acquired(file),
        Err(TryLockError::WouldBlock) => Lock::Held,
        Err(TryLockError::Error(_)) => Lock::Unavailable,
    }
}

/// Asks the running instance to show its window. It may still be starting
/// up, so this tries for a few seconds.
pub fn activate_running() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let conn = zbus::Connection::session()
            .await
            .map_err(|e| e.to_string())?;
        let mut last = String::new();
        for _ in 0..20 {
            let platform_data: HashMap<&str, Value<'_>> = HashMap::new();
            match conn
                .call_method(
                    Some(APP_ID),
                    OBJECT_PATH,
                    Some(IFACE),
                    "Activate",
                    &(platform_data,),
                )
                .await
            {
                Ok(_) => return Ok(()),
                Err(e) => last = e.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        Err(last)
    })
}

struct Application {
    events: async_channel::Sender<Event>,
}

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    async fn activate(&self, _platform_data: HashMap<String, OwnedValue>) {
        let _ = self.events.send(Event::Tray(TrayAction::Show)).await;
    }
}

/// Owns the app's D-Bus name and serves `Activate` until the app exits.
/// Runs on the worker's tokio runtime.
pub async fn serve(events: async_channel::Sender<Event>) {
    let result = async {
        let conn = zbus::connection::Builder::session()?
            .serve_at(OBJECT_PATH, Application { events })?
            .build()
            .await?;
        conn.request_name_with_flags(APP_ID, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .await?;
        Ok::<_, zbus::Error>(conn)
    }
    .await;
    match result {
        // Keep the connection, and with it the name, for the app's lifetime.
        Ok(conn) => {
            std::future::pending::<()>().await;
            drop(conn);
        }
        Err(e) => eprintln!("single-instance D-Bus name unavailable: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Lock, lock};

    #[test]
    fn second_lock_is_refused_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock(dir.path());
        assert!(matches!(first, Lock::Acquired(_)));
        assert!(matches!(lock(dir.path()), Lock::Held));
        drop(first);
        assert!(matches!(lock(dir.path()), Lock::Acquired(_)));
    }
}
