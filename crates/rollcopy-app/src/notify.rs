//! Desktop notifications (PLAN §6) through org.freedesktop.Notifications.
//!
//! Sent from the worker's tokio runtime over **one D-Bus connection kept
//! open for the app's lifetime**. Cinnamon destroys an app's notifications as
//! soon as their sender disconnects (`_onNameVanished` in its
//! notificationDaemon.js), so a connection per notification made popups
//! vanish instantly: the sound played, nothing showed (found 2026-11-12).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use zbus::Variant;

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
        let clicks: Arc<Mutex<VecDeque<(u32, OnClick)>>> = Arc::default();
        tokio::spawn(listen_for_clicks(conn.clone(), clicks.clone(), events));
        while let Ok(note) = rx.recv().await {
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
            let reply = conn
                .call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    &args,
                )
                .await;
            match reply.and_then(|m| m.body().deserialize::<u32>()) {
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

/// Acts on `ActionInvoked` for our notifications.
async fn listen_for_clicks(
    conn: zbus::Connection,
    clicks: Arc<Mutex<VecDeque<(u32, OnClick)>>>,
    events: async_channel::Sender<Event>,
) {
    use futures_lite::StreamExt as _;
    let Ok(rule) = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.Notifications")
        .member("ActionInvoked")
        .build()
    else {
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
        let Ok((id, _action)) = msg.body().deserialize::<(u32, String)>() else {
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
