//! What the main window does with events from the worker, the queue and
//! the tray: library loads and reloads, the copied-state scan, the Hidden
//! album, notifications and the tray icon.

use std::collections::{HashMap, HashSet};

use gpui_kit::*;
use rollcopy_device::{NoDeviceHint, Phase};
use rollcopy_library::Library;

use crate::tree::{LibraryTree, Selection};
use crate::worker::{Command, Event};

use crate::view::*;

impl MainView {
    pub(crate) fn on_event(&mut self, ev: Event, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            Event::TrayAvailable(ok) => self.tray_available = ok,
            Event::FullImage { key, result } => {
                let state = match result {
                    Ok(p) => crate::viewer::Full::Ready(p),
                    Err(e) => crate::viewer::Full::Failed(e),
                };
                self.full.insert(key, state);
            }
            Event::FullImageDropped { key } => {
                if matches!(self.full.get(&key), Some(crate::viewer::Full::Pending)) {
                    self.full.remove(&key);
                }
            }
            Event::Tray(action) => self.on_tray_action(action, window, cx),
            Event::Phase(p) => {
                crate::applog!("phase: {}", phase_label(&p));
                let same_phone = p.udid().is_some() && p.udid() == self.phase.udid();
                if !matches!(p, Phase::Ready { .. }) || !same_phone {
                    self.reloading = false;
                    self.revealed.clear();
                    self.library = LibState::None;
                    self.selection = Selection::All;
                    self.show_details = false;
                    self.status = None;
                    self.thumbs.clear();
                    self.recompute_visible();
                }
                if matches!(p, Phase::Ready { .. }) {
                    self.phase = p;
                    self.on_device_ready(cx);
                } else {
                    self.phase = p;
                }
            }
            Event::Queue(ev) => self.on_queue_event(ev),
            Event::LibraryLoading => {
                if !(self.reloading && matches!(self.library, LibState::Loaded { .. })) {
                    self.library = LibState::Loading;
                }
            }
            Event::Library(Ok(mut library)) => {
                dev_repeat(&mut library);
                let tree = LibraryTree::build(&library.assets, self.config.library.show_hidden);
                self.keep_album_selection(&tree);
                let reloaded = std::mem::take(&mut self.reloading);
                // After a reload, keep the place: indices change, device
                // paths don't.
                let focused = self
                    .focused
                    .and_then(|i| self.assets().get(i))
                    .map(|a| a.device_path());
                if reloaded {
                    let n = library.assets.len() as isize - self.assets().len() as isize;
                    self.status = Some(match n {
                        0 => "Up to date".to_owned(),
                        n if n > 0 => format!("{n} new item{}", if n == 1 { "" } else { "s" }),
                        n => format!("{} item{} removed", -n, if n == -1 { "" } else { "s" }),
                    });
                } else {
                    self.expanded.clear();
                    if let Some(newest) = tree.years.first() {
                        self.expanded.insert(newest.year);
                    }
                }
                self.library = LibState::Loaded { library, tree };
                self.focused = focused
                    .and_then(|key| self.assets().iter().position(|a| a.device_path() == key));
                let keys: HashSet<String> = self.assets().iter().map(|a| a.device_path()).collect();
                self.selected.retain(|k| keys.contains(k));
                self.forget_hidden_thumbs(cx);
                self.refresh_copied(cx);
                self.counts_dirty = true;
                self.recompute_visible();
            }
            Event::Library(Err(e)) => {
                crate::applog!("library load failed: {e}");
                if std::mem::take(&mut self.reloading)
                    && matches!(self.library, LibState::Loaded { .. })
                {
                    self.status = Some(format!("Couldn't reload: {e}"));
                } else {
                    self.library = LibState::Failed(e);
                }
            }
            Event::Thumb { key, path } => {
                self.thumbs.insert(key, Thumb::Ready(path));
            }
            Event::ThumbMissing { key } => {
                self.thumbs.insert(key, Thumb::Missing);
            }
            Event::Opened { name, app, hint } => {
                let mut s = match app {
                    Some(app) => format!("Opened {name} in {app}"),
                    None => format!("Opened {name}"),
                };
                if let Some(hint) = hint {
                    crate::applog!("open: player may lack video decoders");
                    s.push_str(". If it doesn't play: ");
                    s.push_str(&hint);
                }
                self.status = Some(s);
            }
            Event::OpenFailed { name, error } => {
                self.status = Some(format!("Couldn't open {name}: {error}"))
            }
        }
    }

    /// Reads the phone's library again, keeping the current list on screen
    /// until the new one arrives (F5).
    pub(crate) fn reload_library(&mut self) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        if self.reloading || !matches!(self.library, LibState::Loaded { .. }) {
            return;
        }
        self.reloading = true;
        self.status = Some("Reloading from the iPhone…".into());
        let _ = self.commands.try_send(Command::Reload { udid });
    }

    /// Album indices change when the tree is rebuilt: keep the selected album
    /// by name, or fall back to All media if it's gone.
    pub(crate) fn keep_album_selection(&mut self, new: &LibraryTree) {
        let Selection::Album(i) = self.selection else {
            return;
        };
        let name = match &self.library {
            LibState::Loaded { tree, .. } => tree.albums.get(i).map(|(n, _)| n.clone()),
            _ => None,
        };
        self.selection = name
            .and_then(|n| new.album_index(&n))
            .map_or(Selection::All, Selection::Album);
    }

    /// Rebuilds the tree after the Hidden-album preference changed.
    pub(crate) fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let show_hidden = self.config.library.show_hidden;
        let new = match &self.library {
            LibState::Loaded { library, .. } => LibraryTree::build(&library.assets, show_hidden),
            _ => return,
        };
        self.keep_album_selection(&new);
        if let LibState::Loaded { tree, .. } = &mut self.library {
            *tree = new;
        }
        self.forget_hidden_thumbs(cx);
        self.counts_dirty = true;
        self.recompute_visible();
    }

    /// With the Hidden album not shown, deletes cached thumbnails of its
    /// items (some may be cached from before, or from when it was shown).
    pub(crate) fn forget_hidden_thumbs(&mut self, cx: &mut Context<Self>) {
        if self.config.library.show_hidden {
            return;
        }
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        let mut files = Vec::new();
        let mut keys = Vec::new();
        for a in self.assets().iter().filter(|a| a.hidden) {
            files.push(self.paths.thumb(&udid, a));
            files.push(self.paths.veil(&udid, a));
            files.push(self.paths.view(&udid, a));
            keys.push(a.device_path());
        }
        self.revealed.clear();
        for key in keys {
            self.thumbs.remove(&key);
            self.full.remove(&key);
        }
        if files.is_empty() {
            return;
        }
        cx.background_executor()
            .spawn(async move {
                for f in files {
                    let _ = std::fs::remove_file(f);
                }
            })
            .detach();
    }

    /// Works out which items are already in the destination, on a
    /// background thread (one or two file checks per item; on a network or
    /// sleeping USB disk that froze the window, audit PERF-02).
    pub(crate) fn refresh_copied(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.copy_settings() else {
            self.in_dest.clear();
            self.copied_for = None;
            return;
        };
        self.copied_for = Some(settings.clone());
        self.copied_gen += 1;
        let generation = self.copied_gen;
        let assets: Vec<rollcopy_library::Asset> = self
            .assets()
            .iter()
            .filter(|a| !a.trashed)
            .cloned()
            .collect();
        let scan = cx.background_executor().spawn(async move {
            assets
                .iter()
                .map(|a| (a.device_path(), rollcopy_queue::find_copy(&settings, a)))
                .collect::<HashMap<_, _>>()
        });
        cx.spawn(async move |this, cx| {
            let found = scan.await;
            let _ = this.update(cx, |view, cx| {
                if view.copied_gen != generation {
                    return;
                }
                for (key, place) in found {
                    // A copy that finished during the scan stays known.
                    if place.is_some() || !matches!(view.in_dest.get(&key), Some(Some(_))) {
                        view.in_dest.insert(key, place);
                    }
                }
                view.counts_dirty = true;
                view.recompute_visible();
                cx.notify();
            });
        })
        .detach();
    }

    /// Runs after every worker event: tray state, finished-batch handling.
    pub(crate) fn after_event(&mut self, window: &Window, cx: &App) {
        self.sync_tray(cx);
        // Pause/resume (from the tray, the window, or a disconnect) notifies
        // when Rollcopy isn't in front.
        if self.transfers.paused != self.notified_pause {
            let in_background = self.closed_to_tray || !window.is_window_active();
            if self.config.app.notify_when_done && in_background {
                match &self.transfers.paused {
                    Some(reason) => crate::notify::send("Copying paused", reason.clone()),
                    None => crate::notify::send("Copying resumed", String::new()),
                }
            }
            self.notified_pause = self.transfers.paused.clone();
        }
        let active = self.unfinished_transfers() > 0;
        if active && !self.was_active {
            let done = crate::transfers::overall(&self.transfers.rows, None).map_or(0, |o| o.done);
            self.transfers.batch_start = Some((std::time::Instant::now(), done));
        }
        if self.was_active && !active {
            self.transfers.batch_start = None;
            self.on_batch_finished(window);
        }
        self.was_active = active;
    }

    pub(crate) fn on_batch_finished(&mut self, window: &Window) {
        use rollcopy_queue::JobState;
        let rows = &self.transfers.rows;
        let copied = rows
            .iter()
            .filter(|r| matches!(r.state, JobState::Done | JobState::Overwritten))
            .count();
        let failed = rows
            .iter()
            .filter(|r| matches!(r.state, JobState::Failed { .. }))
            .count();
        let in_background = self.closed_to_tray || !window.is_window_active();
        if self.config.app.notify_when_done && in_background {
            let mut body = format!("{copied} copied");
            if failed > 0 {
                body.push_str(&format!(" · {failed} failed"));
            }
            let dest = self.dest_root();
            if let Some(dest) = &dest {
                body.push_str(&format!("\n{}", dest.display()));
            }
            // Click: the folder with the copies, or the window if something
            // failed (the panel says what).
            let click = match dest {
                Some(dest) if failed == 0 && copied > 0 => crate::notify::OnClick::OpenFolder(dest),
                _ => crate::notify::OnClick::ShowWindow,
            };
            crate::notify::send_with("Transfers finished", body, click);
        }
        // Closed during the copy: Rollcopy stays in the tray (decided
        // 2026-11-12). Quitting here used to cut off the notification.
    }

    pub(crate) fn on_tray_action(
        &mut self,
        action: crate::tray::TrayAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::tray::TrayAction;
        match action {
            TrayAction::Show => {
                self.closed_to_tray = false;
                window.activate_window();
            }
            TrayAction::TogglePause => {
                let cmd = if self.transfers.paused.is_some() {
                    rollcopy_queue::QueueCommand::Resume
                } else {
                    rollcopy_queue::QueueCommand::Pause
                };
                let _ = self.commands.try_send(crate::worker::Command::Queue(cmd));
            }
            TrayAction::Quit => self.quit(window, cx),
        }
    }

    /// Sends the tray what it should show, if that changed.
    pub(crate) fn sync_tray(&mut self, cx: &App) {
        use crate::tray::{TrayState, TrayStatus};
        use rollcopy_queue::JobState;
        let rows = &self.transfers.rows;
        let unfinished = self.unfinished_transfers();
        let failed = rows
            .iter()
            .filter(|r| matches!(r.state, JobState::Failed { .. }))
            .count();
        let done = rows.iter().filter(|r| r.state.is_finished()).count();
        let device = match &self.phase {
            Phase::Ready { info } => Some(info.name.clone()),
            _ => None,
        };
        let (status, detail) = if unfinished > 0 {
            let current = rows
                .iter()
                .find(|r| {
                    matches!(
                        r.state,
                        JobState::Reading | JobState::Converting | JobState::Verifying
                    )
                })
                .map(|r| format!(" · {}", r.name))
                .unwrap_or_default();
            let line = match &self.transfers.paused {
                Some(reason) => format!("Paused: {reason}"),
                None => {
                    let left = crate::transfers::overall(rows, self.transfers.batch_start)
                        .and_then(|o| o.left)
                        .map(|l| format!(" · about {} left", crate::transfers::left_label(l)))
                        .unwrap_or_default();
                    format!("Copying {done} of {}{current}{left}", rows.len())
                }
            };
            (TrayStatus::Copying, line)
        } else if failed > 0 {
            (
                TrayStatus::Error,
                format!("{failed} item(s) failed to copy"),
            )
        } else if let Some(name) = &device {
            (TrayStatus::Connected, format!("{name} connected"))
        } else {
            (TrayStatus::Idle, "No iPhone connected".to_owned())
        };
        let state = TrayState {
            status,
            detail,
            device,
            active: unfinished > 0,
            paused: self.transfers.paused.is_some(),
            light_panel: !gpui_kit::component::ActiveTheme::theme(cx).is_dark(),
        };
        if self.last_tray.as_ref() != Some(&state) {
            let _ = self.tray_tx.try_send(state.clone());
            self.last_tray = Some(state);
        }
    }
}

/// Development aid: `ROLLCOPY_DEV_REPEAT=N` repeats the library N times to
/// test grid performance at PLAN §6's 5,000 items with a small phone.
/// Repeated items share device paths, so selection treats them as one.
/// Debug builds only (audit MNT-09).
pub(crate) fn dev_repeat(library: &mut Library) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(n) = std::env::var("ROLLCOPY_DEV_REPEAT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 1)
    else {
        return;
    };
    let original = library.assets.clone();
    for _ in 1..n {
        library.assets.extend(original.iter().cloned());
    }
    crate::applog!(
        "dev: library repeated {n}x = {} items",
        library.assets.len()
    );
}

/// Short phase name for the log (stderr).
pub(crate) fn phase_label(p: &Phase) -> &'static str {
    match p {
        Phase::UsbmuxdMissing => "usbmuxd-missing",
        Phase::NoDevice { hint: None } => "no-device",
        Phase::NoDevice {
            hint: Some(NoDeviceHint::Flaky),
        } => "no-device-flaky",
        Phase::NoDevice {
            hint: Some(NoDeviceHint::AppleUsbNotListed),
        } => "no-device-not-listed",
        Phase::Connecting { .. } => "connecting",
        Phase::WaitingForTrust { .. } => "waiting-for-trust",
        Phase::TrustDenied { .. } => "trust-denied",
        Phase::Locked { .. } => "locked",
        Phase::Ready { .. } => "ready",
        Phase::Failed { .. } => "failed",
    }
}
