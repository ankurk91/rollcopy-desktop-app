//! Tray icon (PLAN §6) over StatusNotifierItem with `ksni`. gpui has no
//! tray support, so this runs on the worker's tokio runtime and talks to the
//! UI through channels.

use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, ToolTip, TrayMethods as _};

use crate::icons;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    #[default]
    Idle,
    Connected,
    Copying,
    Error,
}

impl TrayStatus {
    fn dot(self) -> Option<[u8; 3]> {
        match self {
            TrayStatus::Idle => None,
            TrayStatus::Connected => Some([34, 197, 94]),
            TrayStatus::Copying => Some([59, 130, 246]),
            TrayStatus::Error => Some([239, 68, 68]),
        }
    }
}

/// What the tray shows; sent by the UI whenever it changes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayState {
    pub status: TrayStatus,
    /// Second tooltip line, e.g. "Copying 12 of 40 · IMG_0042.MP4".
    pub detail: String,
    pub device: Option<String>,
    /// Copies are queued or running (Pause/Resume makes sense).
    pub active: bool,
    pub paused: bool,
}

/// Menu and click actions, sent to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    TogglePause,
    Quit,
}

struct RollcopyTray {
    state: TrayState,
    icons: [Vec<Icon>; 4],
    actions: async_channel::Sender<TrayAction>,
}

fn to_ksni(icons: Vec<icons::Argb>) -> Vec<Icon> {
    icons
        .into_iter()
        .map(|i| Icon {
            width: i.width as i32,
            height: i.height as i32,
            data: i.data,
        })
        .collect()
}

impl RollcopyTray {
    fn send(&self, a: TrayAction) {
        let _ = self.actions.try_send(a);
    }

    fn icon_index(&self) -> usize {
        match self.state.status {
            TrayStatus::Idle => 0,
            TrayStatus::Connected => 1,
            TrayStatus::Copying => 2,
            TrayStatus::Error => 3,
        }
    }
}

impl ksni::Tray for RollcopyTray {
    fn id(&self) -> String {
        "io.github.ankurk91.Rollcopy".into()
    }

    fn title(&self) -> String {
        "Rollcopy".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons[self.icon_index()].clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Rollcopy".into(),
            description: self.state.detail.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayAction::Show);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let device = self
            .state
            .device
            .clone()
            .unwrap_or_else(|| "No iPhone connected".into());
        vec![
            StandardItem {
                label: "Show Rollcopy".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayAction::Show)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: device,
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if self.state.paused {
                    "Resume copying".into()
                } else {
                    "Pause copying".into()
                },
                // Resume must always work, even if the queue went idle
                // while paused (otherwise copying stays paused for good).
                enabled: self.state.active || self.state.paused,
                activate: Box::new(|t: &mut Self| t.send(TrayAction::TogglePause)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Starts the tray on the current tokio runtime and applies state updates
/// until the UI goes away. Returns `false` (and does nothing) when no tray
/// host is available; the UI then keeps the "still copying" prompt.
pub async fn run(
    updates: async_channel::Receiver<TrayState>,
    actions: async_channel::Sender<TrayAction>,
) -> bool {
    let icons = [
        to_ksni(icons::tray_icons(TrayStatus::Idle.dot())),
        to_ksni(icons::tray_icons(TrayStatus::Connected.dot())),
        to_ksni(icons::tray_icons(TrayStatus::Copying.dot())),
        to_ksni(icons::tray_icons(TrayStatus::Error.dot())),
    ];
    let tray = RollcopyTray {
        state: TrayState::default(),
        icons,
        actions,
    };
    let handle = match tray.spawn().await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            return false;
        }
    };
    tokio::spawn(async move {
        while let Ok(state) = updates.recv().await {
            if handle.update(|t| t.state = state).await.is_none() {
                break;
            }
        }
    });
    true
}
