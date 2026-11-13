//! Copying (PLAN §6 copy actions, bottom transfer queue, §7.5 conflict
//! dialog): starting copies, the destination folder, the queue panel and the
//! "already exists" prompt. The work itself runs in `rollcopy-queue`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::Asset;
use rollcopy_queue::{JobId, JobState, QueueCommand, QueueEvent, Settings};

use crate::view::{MainView, btn, muted};
use crate::worker::Command;

const PANEL_H: f32 = 220.;
const ROW_H: f32 = 28.;

pub(crate) struct TransferRow {
    pub id: JobId,
    pub name: String,
    pub source: String,
    pub dest: PathBuf,
    pub state: JobState,
    pub progress: Option<(u64, u64)>,
    /// Local time the item finished (done, skipped or failed).
    pub finished_at: Option<chrono::DateTime<chrono::Local>>,
}

pub(crate) struct ConflictPrompt {
    pub id: JobId,
    pub name: String,
    pub dest: PathBuf,
    pub existing_size: u64,
    pub existing_modified: Option<SystemTime>,
    pub new_size: Option<u64>,
    pub others: usize,
    pub apply_to_all: bool,
}

/// Transfer-related state held by the main view.
#[derive(Default)]
pub(crate) struct Transfers {
    pub rows: Vec<TransferRow>,
    pub index: HashMap<JobId, usize>,
    pub paused: Option<String>,
    pub expanded: bool,
    pub conflict: Option<ConflictPrompt>,
    pub scroll: UniformListScrollHandle,
    /// One-line result of the last panel action ("Exported 3 errors to …").
    pub note: Option<String>,
}

impl Transfers {
    fn counts(&self) -> (usize, usize, usize, usize) {
        let mut done = 0;
        let mut failed = 0;
        let mut active = 0;
        for r in &self.rows {
            match r.state {
                JobState::Done
                | JobState::Overwritten
                | JobState::Skipped
                | JobState::Cancelled => done += 1,
                JobState::Failed { .. } => failed += 1,
                _ => active += 1,
            }
        }
        (done, failed, active, self.rows.len())
    }
}

fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.))
    }
}

fn spawn_xdg_open(path: PathBuf) {
    // Reap the child on a thread so it doesn't linger as a zombie.
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open").arg(path).status();
    });
}

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
    pub(crate) fn on_device_ready(&mut self) {
        self.sync_queue_settings();
    }

    /// Tells the queue the current settings (destination etc.).
    pub(crate) fn send_queue_settings(&self) {
        if let Some(s) = self.copy_settings() {
            let _ = self
                .commands
                .try_send(Command::Queue(QueueCommand::SetSettings(s)));
        }
    }

    /// Destination changed: send it and recheck what's already copied there.
    pub(crate) fn sync_queue_settings(&mut self) {
        self.send_queue_settings();
        self.in_dest.clear();
        self.recompute_visible();
    }

    pub(crate) fn copy_assets(&mut self, assets: Vec<Asset>) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        let assets: Vec<Asset> = assets.into_iter().filter(|a| !a.trashed).collect();
        if assets.is_empty() {
            return;
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

    fn export_errors(&mut self, cx: &mut Context<Self>) {
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
            let note = match rollcopy_queue::History::open(&history)
                .map_err(|e| e.to_string())
                .and_then(|h| h.export_csv(&out).map_err(|e| e.to_string()))
            {
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
                t.rows.retain(|r| !ids.contains(&r.id));
                t.index = t.rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
            }
        }
    }

    fn queue_cmd(&self, cmd: QueueCommand) {
        let _ = self.commands.try_send(Command::Queue(cmd));
    }

    // ---- bottom panel ----------------------------------------------

    pub(crate) fn render_transfers(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.transfers.rows.is_empty() {
            return None;
        }
        let theme = cx.theme().clone();
        let t = &self.transfers;
        let (done, failed, active, total) = t.counts();

        let mut summary = format!("{done}/{total} done");
        if failed > 0 {
            summary.push_str(&format!(" · {failed} failed"));
        }
        if let Some(r) = t.rows.iter().find(|r| {
            matches!(
                r.state,
                JobState::Reading | JobState::Converting | JobState::Verifying
            )
        }) {
            summary.push_str(&format!(
                " · {} {}",
                state_label(&r.state, r.progress),
                r.name
            ));
        }

        let paused = t.paused.clone();
        let header = h_flex()
            .id("transfers-header")
            .w_full()
            .px_4()
            .py_1p5()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                this.transfers.expanded = !this.transfers.expanded;
                cx.notify();
            }))
            .child(
                Icon::new(if t.expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall(),
            )
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Transfers"))
            .child(muted(summary, &theme))
            .when_some(paused.clone(), |d, reason| {
                d.child(div().text_sm().text_color(theme.warning).child(reason))
            })
            .child(div().flex_1())
            .when(failed > 0, |d| {
                d.child(
                    btn("retry-all")
                        .xsmall()
                        .ghost()
                        .label("Retry all failed")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.queue_cmd(QueueCommand::RetryAllFailed);
                            cx.stop_propagation();
                        })),
                )
            })
            .when(active > 0 || paused.is_some(), |d| {
                let is_paused = paused.is_some();
                d.child(
                    btn("pause")
                        .xsmall()
                        .outline()
                        .label(if is_paused { "Resume" } else { "Pause" })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.queue_cmd(if is_paused {
                                QueueCommand::Resume
                            } else {
                                QueueCommand::Pause
                            });
                            cx.stop_propagation();
                        })),
                )
            })
            .when(active > 0, |d| {
                d.child(
                    btn("cancel-all")
                        .xsmall()
                        .ghost()
                        .label("Cancel all")
                        .tooltip("Stop copying; files already copied stay")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.queue_cmd(QueueCommand::CancelAll);
                            cx.stop_propagation();
                        })),
                )
            })
            .when(done > 0, |d| {
                d.child(
                    btn("clear")
                        .xsmall()
                        .ghost()
                        .label("Clear finished")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.queue_cmd(QueueCommand::ClearFinished);
                            cx.stop_propagation();
                        })),
                )
            })
            .child(
                btn("export-errors")
                    .xsmall()
                    .ghost()
                    .label("Export error report")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.export_errors(cx);
                        cx.stop_propagation();
                    })),
            )
            .when_some(self.dest_root(), |d, dest| {
                d.child(
                    btn("open-dest")
                        .xsmall()
                        .ghost()
                        .icon(IconName::FolderOpen)
                        .label("Open folder")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            spawn_xdg_open(dest.clone());
                            cx.stop_propagation();
                        })),
                )
            });

        let mut panel = v_flex()
            .w_full()
            .flex_shrink_0()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(header);
        if let Some(note) = t.note.clone() {
            panel = panel.child(div().px_4().pb_1().child(muted(note, &theme)));
        }
        if t.expanded {
            // Column headings, same widths as the rows below.
            panel = panel.child(
                h_flex()
                    .px_4()
                    .pb_1()
                    .gap_3()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(div().w(px(14.)))
                    .child(div().w(px(130.)).flex_shrink_0().child("File"))
                    .child(div().w(px(120.)).flex_shrink_0().child("Folder"))
                    .child(div().flex_1().child("Status"))
                    .child(div().w(px(110.)).flex_shrink_0().child("Finished")),
            );
            panel = panel.child(
                div()
                    .relative()
                    .h(px(PANEL_H))
                    .child(
                        uniform_list(
                            "transfer-rows",
                            t.rows.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|i| this.render_transfer_row(i, cx))
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&t.scroll)
                        .size_full()
                        .px_4()
                        .pb_2(),
                    )
                    .vertical_scrollbar(&t.scroll),
            );
        }
        Some(panel.into_any_element())
    }

    fn render_transfer_row(&self, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let r = &self.transfers.rows[i];
        let (icon, color) = match &r.state {
            JobState::Done => ("✔", theme.success),
            JobState::Overwritten => ("✔", theme.success),
            JobState::Skipped => ("↷", theme.muted_foreground),
            JobState::Cancelled => ("✕", theme.muted_foreground),
            JobState::Failed { .. } => ("✖", theme.danger),
            JobState::Conflict => ("?", theme.warning),
            JobState::Queued => ("•", theme.muted_foreground),
            _ => ("⟳", theme.primary),
        };
        let folder = self
            .dest_root()
            .and_then(|root| r.dest.parent()?.strip_prefix(root).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let id = r.id;
        h_flex()
            .h(px(ROW_H))
            .w_full()
            .gap_3()
            .items_center()
            .text_sm()
            .child(div().w(px(14.)).text_color(color).child(icon))
            .child(div().w(px(130.)).flex_shrink_0().child(r.name.clone()))
            .child(
                div()
                    .w(px(120.))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground)
                    .child(folder),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_color(if matches!(r.state, JobState::Failed { .. }) {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(state_label(&r.state, r.progress)),
            )
            .child(
                div()
                    .w(px(110.))
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        r.finished_at
                            .map(|t| t.format("%-d %b %H:%M:%S").to_string())
                            .unwrap_or_default(),
                    ),
            )
            .when(matches!(r.state, JobState::Failed { .. }), |d| {
                d.child(
                    btn(("retry", id as usize))
                        .xsmall()
                        .outline()
                        .label("Retry")
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.queue_cmd(QueueCommand::Retry(id));
                        })),
                )
            })
            .when(!r.state.is_finished(), |d| {
                d.child(
                    btn(("cancel", id as usize))
                        .xsmall()
                        .ghost()
                        .icon(IconName::X)
                        .tooltip("Cancel")
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.queue_cmd(QueueCommand::Cancel(id));
                        })),
                )
            })
            .into_any_element()
    }

    // ---- conflict prompt -------------------------------------------

    pub(crate) fn render_conflict(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.transfers.conflict.as_ref()?;
        let theme = cx.theme().clone();
        let folder = self
            .dest_root()
            .and_then(|root| c.dest.parent()?.strip_prefix(root).ok())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| {
                c.dest
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            });
        let existing_date = c
            .existing_modified
            .map(|t| {
                chrono::DateTime::<chrono::Local>::from(t)
                    .format("%-d %b %Y")
                    .to_string()
            })
            .unwrap_or_default();
        let new_size = c
            .new_size
            .map(human_size)
            .unwrap_or_else(|| "unknown size".into());
        let (id, apply, others) = (c.id, c.apply_to_all, c.others);

        let card = v_flex()
            .w(px(520.))
            .p_5()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("{} already exists in {folder}", c.name)),
            )
            .child(muted(
                format!(
                    "Existing: {} · {existing_date}    New: {new_size}",
                    human_size(c.existing_size)
                ),
                &theme,
            ))
            .when(others > 0, |d| {
                d.child(
                    Checkbox::new("apply-all")
                        .checked(apply)
                        .label(format!("Do this for the remaining {others} conflicts"))
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(c) = this.transfers.conflict.as_mut() {
                                c.apply_to_all = *checked;
                            }
                            cx.notify();
                        })),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        btn("conflict-skip")
                            .outline()
                            .label("Skip")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_cmd(QueueCommand::Resolve {
                                    id,
                                    overwrite: false,
                                    apply_to_all: apply,
                                });
                                this.transfers.conflict = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        btn("conflict-overwrite")
                            .primary()
                            .label("Overwrite")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.queue_cmd(QueueCommand::Resolve {
                                    id,
                                    overwrite: true,
                                    apply_to_all: apply,
                                });
                                this.transfers.conflict = None;
                                cx.notify();
                            })),
                    ),
            );

        Some(
            div()
                .id("conflict-backdrop")
                .absolute()
                .inset_0()
                .bg(hsla(0., 0., 0., 0.45))
                .flex()
                .items_center()
                .justify_center()
                // Swallow clicks so the grid behind doesn't react.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(card)
                .into_any_element(),
        )
    }

    // ---- toolbar pieces ----------------------------------------------

    pub(crate) fn render_save_to(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dest = self.dest_root()?;
        let theme = cx.theme().clone();
        let n = self.copyable_selected();
        Some(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Save to"),
                )
                .child(
                    div()
                        .max_w(px(340.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_sm()
                        .child(tilde(&dest)),
                )
                .child(
                    btn("pick-dest")
                        .xsmall()
                        .ghost()
                        .icon(IconName::FolderOpen)
                        .tooltip("Choose the destination folder")
                        .on_click(cx.listener(|this, _, _, cx| this.pick_destination(cx))),
                )
                .child(
                    btn("copy-selected")
                        .small()
                        .primary()
                        .icon(IconName::Copy)
                        .label(format!("Copy selected ({n})"))
                        .disabled(n == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.copy_selected();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}

fn tilde(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

pub(crate) fn state_label(s: &JobState, progress: Option<(u64, u64)>) -> String {
    match s {
        JobState::Queued => "Queued".into(),
        JobState::Reading => match progress {
            Some((d, t)) if t > 0 => format!("Reading {}%", d * 100 / t),
            _ => "Reading".into(),
        },
        JobState::Converting => "Converting".into(),
        JobState::Verifying => "Verifying".into(),
        JobState::Done => "Done".into(),
        JobState::Overwritten => "Overwritten".into(),
        JobState::Skipped => "Skipped (already there)".into(),
        JobState::Cancelled => "Cancelled".into(),
        JobState::Failed { stage, error } => format!("Failed ({}): {error}", stage.as_str()),
        JobState::Conflict => "Already exists, waiting for your answer".into(),
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that would bring in gpui's own `test` macro.
    use super::{JobState, human_size, state_label};

    #[test]
    fn labels() {
        assert_eq!(
            state_label(&JobState::Reading, Some((50, 200))),
            "Reading 25%"
        );
        assert_eq!(human_size(23_600_000), "23.6 MB");
    }
}
