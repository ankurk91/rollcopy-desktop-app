//! Desktop notifications (PLAN §6) through org.freedesktop.Notifications,
//! or, inside Flatpak, the notification portal (Flathub asks sandboxed apps
//! not to talk to the notification service directly).
//!
//! Sent from the worker's tokio runtime over **one D-Bus connection kept
//! open for the app's lifetime**. Cinnamon destroys an app's notifications as
//! soon as their sender disconnects (`_onNameVanished` in its
//! notificationDaemon.js), so a connection per notification made popups
//! vanish instantly: the sound played, nothing showed (found 2026-11-12).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use zbus::zvariant::Value as Variant;

use crate::tray::TrayAction;
use crate::worker::Event;

/// How long popups stay up; Cinnamon's default is 4 s.
const EXPIRE_MS: i32 = 10_000;

/// What clicking a notification does.
#[derive(Debug, Clone)]
pub enum OnClick {
    /// Bring the Rollcopy window back.
    ShowWindow,
    /// Open this folder in the file manager.
    OpenFolder(PathBuf),
}

struct Note {
    summary: String,
    body: String,
    click: OnClick,
}

static SENDER: OnceLock<async_channel::Sender<Note>> = OnceLock::new();

/// Queues a notification; clicking it shows the window. Does nothing until
/// [`start`] has run.
pub fn send(summary: impl Into<String>, body: impl Into<String>) {
    send_with(summary, body, OnClick::ShowWindow);
}

/// Queues a notification with a click action.
pub fn send_with(summary: impl Into<String>, body: impl Into<String>, click: OnClick) {
    if let Some(tx) = SENDER.get() {
        let _ = tx.try_send(Note {
            summary: summary.into(),
            body: body.into(),
            click,
        });
    }
}

/// Starts the notifier on the current tokio runtime. Clicks that should
/// show the window are sent to the UI on `events`.
pub fn start(events: async_channel::Sender<Event>) {
    let (tx, rx) = async_channel::unbounded::<Note>();
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
        // Notification id → what a click on it does (the last few only).
        let clicks: Clicks = Arc::default();
        let portal = crate::flatpak::active();
        tokio::spawn(listen_for_clicks(
            conn.clone(),
            clicks.clone(),
            events,
            portal,
        ));
        let mut next = 0u32;
        while let Ok(note) = rx.recv().await {
            let sent = if portal {
                next += 1;
                let id = format!("rollcopy-{next}");
                send_portal(&conn, &id, &note).await.map(|()| id)
            } else {
                send_direct(&conn, &note).await.map(|id| id.to_string())
            };
            match sent {
                Ok(id) => {
                    let mut c = clicks.lock().unwrap_or_else(|e| e.into_inner());
                    c.push_back((id, note.click));
                    while c.len() > 32 {
                        c.pop_front();
                    }
                }
                Err(e) => crate::applog!("notification failed: {e}"),
            }
        }
        // `conn` lives until the app exits, keeping its notifications on screen.
    });
}

/// Recent notification ids and what a click on each does.
type Clicks = Arc<Mutex<VecDeque<(String, OnClick)>>>;

/// `org.freedesktop.Notifications.Notify`; returns the server's id.
async fn send_direct(conn: &zbus::Connection, note: &Note) -> zbus::Result<u32> {
    let hints: HashMap<&str, Variant<'_>> =
        HashMap::from([("desktop-entry", Variant::from(crate::APP_ID))]);
    // "default" is what clicking the notification itself triggers.
    let label = match note.click {
        OnClick::ShowWindow => "Show Rollcopy",
        OnClick::OpenFolder(_) => "Open folder",
    };
    let args = (
        "Rollcopy",
        0u32,
        crate::APP_ID,
        note.summary.as_str(),
        note.body.as_str(),
        vec!["default", label],
        hints,
        EXPIRE_MS,
    );
    conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &args,
    )
    .await?
    .body()
    .deserialize::<u32>()
}

/// `org.freedesktop.portal.Notification.AddNotification` under our own
/// `id`. The portal knows the app (and its icon) from the sandbox; a click
/// comes back as `ActionInvoked` with the "default" action.
async fn send_portal(conn: &zbus::Connection, id: &str, note: &Note) -> zbus::Result<()> {
    let notification: HashMap<&str, Variant<'_>> = HashMap::from([
        ("title", Variant::from(note.summary.as_str())),
        ("body", Variant::from(note.body.as_str())),
        ("default-action", Variant::from("default")),
    ]);
    conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.Notification"),
        "AddNotification",
        &(id, notification),
    )
    .await
    .map(|_| ())
}

/// Acts on `ActionInvoked` for our notifications, from the notification
/// service or, with `portal`, the notification portal.
async fn listen_for_clicks(
    conn: zbus::Connection,
    clicks: Clicks,
    events: async_channel::Sender<Event>,
    portal: bool,
) {
    use futures_lite::StreamExt as _;
    let interface = if portal {
        "org.freedesktop.portal.Notification"
    } else {
        "org.freedesktop.Notifications"
    };
    let rule = (|| {
        Ok::<_, zbus::Error>(
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface(interface)?
                .member("ActionInvoked")?
                .build(),
        )
    })();
    let Ok(rule) = rule else {
        return;
    };
    let mut stream = match zbus::MessageStream::for_match_rule(rule, &conn, None).await {
        Ok(s) => s,
        Err(e) => {
            crate::applog!("notification clicks unavailable: {e}");
            return;
        }
    };
    while let Some(Ok(msg)) = stream.next().await {
        // The service sends (u id, s action); the portal (s id, s action, av).
        let body = msg.body();
        let id = if portal {
            body.deserialize::<(String, String, Vec<zbus::zvariant::OwnedValue>)>()
                .map(|(id, _, _)| id)
        } else {
            body.deserialize::<(u32, String)>()
                .map(|(id, _)| id.to_string())
        };
        let Ok(id) = id else {
            continue;
        };
        let click = {
            let c = clicks.lock().unwrap_or_else(|e| e.into_inner());
            c.iter().find(|(i, _)| *i == id).map(|(_, k)| k.clone())
        };
        match click {
            Some(OnClick::ShowWindow) => {
                let _ = events.send(Event::Tray(TrayAction::Show)).await;
            }
            Some(OnClick::OpenFolder(dir)) => crate::desktop::open_folder(&dir),
            None => {}
        }
    }
}
