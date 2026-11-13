//! Desktop notifications (PLAN §6) through org.freedesktop.Notifications.
//!
//! Sent from the worker's tokio runtime over **one D-Bus connection kept
//! open for the app's lifetime**. Cinnamon destroys an app's notifications as
//! soon as their sender disconnects (`_onNameVanished` in its
//! notificationDaemon.js), so a connection per notification made popups
//! vanish instantly: the sound played, nothing showed (found 2026-11-12).

use std::collections::HashMap;
use std::sync::OnceLock;

use zbus::zvariant::Value;

/// How long popups stay up; Cinnamon's default is 4 s.
const EXPIRE_MS: i32 = 10_000;

static SENDER: OnceLock<async_channel::Sender<(String, String)>> = OnceLock::new();

/// Queues a notification. Does nothing until [`run`] is started.
pub fn send(summary: impl Into<String>, body: impl Into<String>) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.try_send((summary.into(), body.into()));
    }
}

/// Starts the notifier on the current tokio runtime.
pub fn start() {
    let (tx, rx) = async_channel::unbounded::<(String, String)>();
    if SENDER.set(tx).is_err() {
        return;
    }
    tokio::spawn(async move {
        let conn = match zbus::Connection::session().await {
            Ok(c) => c,
            Err(e) => {
                crate::applog!("notifications unavailable: {e}");
                return;
            }
        };
        while let Ok((summary, body)) = rx.recv().await {
            let hints: HashMap<&str, Value<'_>> =
                HashMap::from([("desktop-entry", Value::from(crate::APP_ID))]);
            let args = (
                "Rollcopy",
                0u32,
                crate::APP_ID,
                summary.as_str(),
                body.as_str(),
                Vec::<&str>::new(),
                hints,
                EXPIRE_MS,
            );
            if let Err(e) = conn
                .call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &args,
                )
                .await
            {
                crate::applog!("notification failed: {e}");
            }
        }
        // `conn` lives until the app exits, keeping its notifications on screen.
    });
}
