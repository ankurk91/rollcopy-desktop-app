//! Starting copies, the destination folder, and queue events.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::*;
use rollcopy_library::Asset;
use rollcopy_queue::{JobId, JobState, QueueCommand, QueueEvent, Settings};

use crate::view::MainView;
use crate::worker::Command;

use super::*;

impl MainView {
    // ---- starting copies ---------------------------------------------

    /// Current destination for the connected phone.
    /// Save-to folder for the connected phone (PLAN §8a rules, from the
    /// saved preferences).
    pub(crate) fn dest_root(&self) -> Option<PathBuf> {
        match &self.phase {
            rollcopy_device::Phase::Ready { info } => {
                Some(self.config.destination_for(&info.udid, &info.name))
            }
            _ => None,
        }
    }

    pub(crate) fn copy_settings(&self) -> Option<Settings> {
        self.dest_root().map(|d| self.config.copy_settings(d))
    }

    /// Called when a phone becomes ready: tell the queue where to copy.
    pub(crate) fn on_device_ready(&mut self, cx: &mut Context<Self>) {
        self.sync_queue_settings(cx);
    }

    /// Tells the queue the current settings (destination etc.).
    pub(crate) fn send_queue_settings(&self) {
        if let Some(s) = self.copy_settings() {
            let _ = self
                .commands
                .try_send(Command::Queue(QueueCommand::SetSettings(s)));
        }
    }

    /// Settings may have changed: send them, and if the destination or
    /// layout changed, recheck what's already copied there.
    pub(crate) fn sync_queue_settings(&mut self, cx: &mut Context<Self>) {
        self.send_queue_settings();
        if self.copy_settings() != self.copied_for {
            self.in_dest.clear();
            self.refresh_copied(cx);
            self.recompute_visible();
        }
    }

    pub(crate) fn copy_assets(&mut self, assets: Vec<Asset>) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        let mut assets: Vec<Asset> = assets.into_iter().filter(|a| !a.trashed).collect();
        if assets.is_empty() {
            return;
        }
        // Live Photos' motion parts, right after their photos, if wanted.
        if self.config.conversion.live_motion {
            assets = assets
                .into_iter()
                .flat_map(|a| {
                    let motion = a.live_motion();
                    std::iter::once(a).chain(motion)
                })
                .collect();
        }
        self.send_queue_settings();
        let _ = self
            .commands
            .try_send(Command::Queue(QueueCommand::Add { udid, assets }));
    }

    /// Selected items in grid order.
    pub(crate) fn copy_selected(&mut self) {
        let assets: Vec<Asset> = self
            .visible
            .iter()
            .map(|&i| &self.assets()[i])
            .filter(|a| self.selected.contains(&a.device_path()))
            .cloned()
            .collect();
        self.copy_assets(assets);
    }

    /// Everything shown in the grid (the selected year/month and filter).
    pub(crate) fn copy_visible(&mut self) {
        let assets: Vec<Asset> = self
            .visible
            .iter()
            .map(|&i| self.assets()[i].clone())
            .collect();
        self.copy_assets(assets);
    }

    /// Everything on the phone not copied yet, whatever is shown: no
    /// Hidden or Recently Deleted items. Only after the copied-state scan
    /// has run, so nothing already copied is queued again.
    pub(crate) fn new_items(&self) -> Vec<Asset> {
        if self.copied_for.is_none() {
            return Vec::new();
        }
        self.assets()
            .iter()
            .filter(|a| crate::filters::is_new(a, &self.in_dest))
            .cloned()
            .collect()
    }

    pub(crate) fn copy_new(&mut self) {
        let mut assets = self.new_items();
        assets.sort_by_key(|a| a.captured);
        self.copy_assets(assets);
    }

    pub(crate) fn copy_focused(&mut self) {
        if let Some(a) = self.focused.and_then(|i| self.assets().get(i)).cloned() {
            self.copy_assets(vec![a]);
        }
    }

    pub(crate) fn copyable_selected(&self) -> usize {
        self.visible
            .iter()
            .map(|&i| &self.assets()[i])
            .filter(|a| !a.trashed && self.selected.contains(&a.device_path()))
            .count()
    }

    pub(crate) fn pick_destination(&mut self, cx: &mut Context<Self>) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Save here".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(dir) = paths.into_iter().next()
            {
                // Saved per phone; the Prefs observer re-syncs the view.
                let _ = this.update(cx, |_, cx| {
                    crate::prefs::Prefs::update_config(cx, |c| {
                        c.destination.devices.insert(udid, dir);
                    });
                });
            }
        })
        .detach();
    }

    pub(super) fn export_errors(&mut self, cx: &mut Context<Self>) {
        let history = self.paths.history();
        let dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(Path::to_path_buf))
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let rx = cx.prompt_for_new_path(&dir, Some("rollcopy-errors.csv"));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(out))) = rx.await else {
                return;
            };
            // SQLite and file I/O off the UI thread (audit PERF-02).
            let export = cx.background_executor().spawn({
                let out = out.clone();
                async move {
                    rollcopy_queue::History::open(&history)
                        .map_err(|e| e.to_string())
                        .and_then(|h| h.export_csv(&out).map_err(|e| e.to_string()))
                }
            });
            let note = match export.await {
                Ok(n) => format!("Exported {n} error(s) to {}", out.display()),
                Err(e) => format!("Couldn't export: {e}"),
            };
            let _ = this.update(cx, |view, cx| {
                view.transfers.note = Some(note);
                cx.notify();
            });
        })
        .detach();
    }

    // ---- queue events ----------------------------------------------

    pub(crate) fn on_queue_event(&mut self, ev: QueueEvent) {
        let t = &mut self.transfers;
        match ev {
            QueueEvent::Added {
                id,
                name,
                source,
                dest,
                size,
            } => {
                t.index.insert(id, t.rows.len());
                t.rows.push(TransferRow {
                    id,
                    name,
                    source,
                    dest,
                    state: JobState::Queued,
                    progress: None,
                    finished_at: None,
                    started: None,
                    took: None,
                    size,
                });
                t.expanded = true;
                t.note = None;
            }
            QueueEvent::State { id, state } => {
                if let Some(&i) = t.index.get(&id) {
                    let row = &mut t.rows[i];
                    if matches!(state, JobState::Done | JobState::Overwritten) {
                        self.in_dest
                            .insert(row.source.clone(), Some(row.dest.clone()));
                        self.counts_dirty = true;
                    }
                    if !matches!(state, JobState::Reading) {
                        row.progress = None;
                    }
                    row.finished_at = state.is_finished().then(chrono::Local::now);
                    match &state {
                        JobState::Queued | JobState::Conflict => {
                            row.started = None;
                            row.took = None;
                        }
                        JobState::Reading | JobState::Converting | JobState::Verifying => {
                            row.started.get_or_insert_with(std::time::Instant::now);
                        }
                        _ => row.took = row.started.map(|s| s.elapsed()),
                    }
                    row.state = state;
                }
            }
            QueueEvent::Progress { id, done, total } => {
                if let Some(&i) = t.index.get(&id) {
                    t.rows[i].progress = Some((done, total));
                }
            }
            QueueEvent::Retarget { id, dest } => {
                if let Some(&i) = t.index.get(&id) {
                    let row = &mut t.rows[i];
                    if let Some(name) = dest.file_name() {
                        row.name = name.to_string_lossy().into_owned();
                    }
                    row.dest = dest;
                }
            }
            QueueEvent::Paused { reason } => t.paused = reason,
            QueueEvent::Conflict {
                id,
                name,
                dest,
                existing_size,
                existing_modified,
                new_size,
                others,
            } => {
                t.conflict = Some(ConflictPrompt {
                    id,
                    name,
                    dest,
                    existing_size,
                    existing_modified,
                    new_size,
                    others,
                    apply_to_all: false,
                });
            }
            QueueEvent::ConflictCleared => t.conflict = None,
            QueueEvent::Cleared { ids } => {
                let ids: HashSet<JobId> = ids.into_iter().collect();
                t.rows.retain(|r| !ids.contains(&r.id));
                t.index = t.rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
            }
        }
    }

    pub(super) fn queue_cmd(&self, cmd: QueueCommand) {
        let _ = self.commands.try_send(Command::Queue(cmd));
    }
}
