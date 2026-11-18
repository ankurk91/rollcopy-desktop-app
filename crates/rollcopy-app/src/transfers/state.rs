//! Transfer state: rows, overall progress, labels.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::*;
use rollcopy_queue::{JobId, JobState};

/// Height of the transfer list, by default and at least.
pub(crate) const PANEL_H: f32 = 220.;
pub(crate) const PANEL_MIN_H: f32 = 80.;
pub(super) const ROW_H: f32 = 28.;
/// Panel header and column titles above the list.
pub(super) const PANEL_CHROME_H: f32 = 64.;

/// Drag payload for the transfers panel's top edge.
#[derive(Clone)]
pub(crate) struct ResizeTransfers;

impl Render for ResizeTransfers {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub(crate) struct TransferRow {
    pub id: JobId,
    pub name: String,
    pub source: String,
    pub dest: PathBuf,
    pub state: JobState,
    pub progress: Option<(u64, u64)>,
    /// Local time the item finished (done, skipped or failed).
    pub finished_at: Option<chrono::DateTime<chrono::Local>>,
    /// When the item started reading (reset if it goes back in the queue).
    pub started: Option<std::time::Instant>,
    /// How long it took, from starting to read until it finished.
    pub took: Option<std::time::Duration>,
    /// Size of the original, for overall progress.
    pub size: Option<u64>,
}

/// Progress of the copies in the panel, by bytes read from the phone.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Overall {
    pub done: u64,
    pub total: u64,
    /// Estimated time left, once there's enough to go on.
    pub left: Option<std::time::Duration>,
}

impl Overall {
    pub(crate) fn fraction(&self) -> f32 {
        if self.total == 0 {
            0.
        } else {
            (self.done as f64 / self.total as f64).clamp(0., 1.) as f32
        }
    }

    /// "1.2 GB of 3.4 GB · about 4 min left".
    pub(crate) fn label(&self) -> String {
        let mut s = format!("{} of {}", human_size(self.done), human_size(self.total));
        if let Some(left) = self.left {
            s.push_str(&format!(" · about {} left", left_label(left)));
        }
        s
    }
}

/// "under a minute", "4 min", "1 h 05 min".
pub(crate) fn left_label(d: std::time::Duration) -> String {
    let m = (d.as_secs_f64() / 60.).round() as u64;
    match m {
        0 => "under a minute".to_owned(),
        m if m < 60 => format!("{m} min"),
        m => format!("{} h {:02} min", m / 60, m % 60),
    }
}

/// Works out overall progress for rows that are part of the current batch
/// (not skipped or cancelled): finished items count fully, a running one
/// by bytes read. The estimate uses the speed since the batch started and
/// needs 3 s and 1 MB of history.
pub(crate) fn overall(
    rows: &[TransferRow],
    batch_start: Option<(std::time::Instant, u64)>,
) -> Option<Overall> {
    let mut done = 0u64;
    let mut total = 0u64;
    for r in rows {
        let size = r.size.unwrap_or(0);
        match &r.state {
            JobState::Skipped | JobState::Cancelled => continue,
            JobState::Done | JobState::Overwritten | JobState::Failed { .. } => done += size,
            JobState::Reading => done += r.progress.map_or(0, |(d, _)| d.min(size)),
            JobState::Converting | JobState::Verifying => done += size,
            JobState::Queued | JobState::Conflict => {}
        }
        total += size;
    }
    if total == 0 {
        return None;
    }
    let left = batch_start.and_then(|(since, at)| {
        let secs = since.elapsed().as_secs_f64();
        let moved = done.saturating_sub(at);
        (secs >= 3. && moved >= 1_000_000).then(|| {
            let rate = moved as f64 / secs;
            std::time::Duration::from_secs_f64((total - done) as f64 / rate)
        })
    });
    Some(Overall { done, total, left })
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
    /// When the current batch of copies started, and the bytes already
    /// done then (for the time-left estimate).
    pub batch_start: Option<(std::time::Instant, u64)>,
    pub conflict: Option<ConflictPrompt>,
    pub scroll: UniformListScrollHandle,
    /// One-line result of the last panel action ("Exported 3 errors to …").
    pub note: Option<String>,
    /// Column widths (see `columns::TRANSFERS`).
    pub widths: Vec<f32>,
    /// Height of the transfer list.
    pub height: f32,
    /// Width of a row this frame, for fitting the columns.
    pub row_w: f32,
}

impl Transfers {
    pub(super) fn counts(&self) -> (usize, usize, usize, usize) {
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

/// The dock's headline: "Copying 3 of 40", "Paused at 2 of 40",
/// "All 40 done", "38 done · 2 failed". `done` counts skipped and
/// cancelled items too: they're finished.
pub(crate) fn headline(
    done: usize,
    failed: usize,
    active: usize,
    total: usize,
    paused: bool,
) -> String {
    let finished = done + failed;
    if active > 0 && paused {
        format!("Paused at {finished} of {total}")
    } else if active > 0 {
        format!("Copying {} of {total}", (finished + 1).min(total))
    } else if failed == 0 {
        format!("All {total} done")
    } else {
        format!("{done} done · {failed} failed")
    }
}

pub(crate) fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.))
    }
}

pub(super) fn spawn_xdg_open(path: PathBuf) {
    // Reap the child on a thread so it doesn't linger as a zombie.
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open").arg(path).status();
    });
}

pub(super) fn tilde(p: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}

/// "0.8 s", "14 s", "2 min 05 s".
pub(crate) fn took_label(d: std::time::Duration) -> String {
    let s = d.as_secs_f64();
    if s < 10. {
        format!("{s:.1} s")
    } else if s < 60. {
        format!("{s:.0} s")
    } else {
        let s = s.round() as u64;
        format!("{} min {:02} s", s / 60, s % 60)
    }
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

    fn row(state: JobState, size: u64, progress: Option<(u64, u64)>) -> super::TransferRow {
        super::TransferRow {
            id: 0,
            name: String::new(),
            source: String::new(),
            dest: std::path::PathBuf::new(),
            state,
            progress,
            finished_at: None,
            started: None,
            took: None,
            size: Some(size),
        }
    }

    #[test]
    fn overall_progress_by_bytes() {
        let rows = vec![
            row(JobState::Done, 100, None),
            row(JobState::Reading, 200, Some((50, 200))),
            row(JobState::Queued, 300, None),
            row(JobState::Skipped, 999, None),
        ];
        let p = super::overall(&rows, None).unwrap();
        assert_eq!((p.done, p.total), (150, 600));
        assert_eq!(p.fraction(), 0.25);
        assert_eq!(p.left, None);
        assert_eq!(super::overall(&[], None), None);
    }

    #[test]
    fn time_left_labels() {
        use std::time::Duration;
        assert_eq!(super::left_label(Duration::from_secs(20)), "under a minute");
        assert_eq!(super::left_label(Duration::from_secs(250)), "4 min");
        assert_eq!(super::left_label(Duration::from_secs(3900)), "1 h 05 min");
    }

    #[test]
    fn dock_headline() {
        use super::headline;
        assert_eq!(headline(2, 0, 38, 40, false), "Copying 3 of 40");
        assert_eq!(headline(2, 0, 38, 40, true), "Paused at 2 of 40");
        assert_eq!(headline(40, 0, 0, 40, false), "All 40 done");
        assert_eq!(headline(38, 2, 0, 40, false), "38 done · 2 failed");
        assert_eq!(headline(1, 0, 0, 1, false), "All 1 done");
    }

    #[test]
    fn labels() {
        assert_eq!(
            state_label(&JobState::Reading, Some((50, 200))),
            "Reading 25%"
        );
        assert_eq!(human_size(23_600_000), "23.6 MB");
        use std::time::Duration;
        assert_eq!(super::took_label(Duration::from_millis(820)), "0.8 s");
        assert_eq!(super::took_label(Duration::from_secs(14)), "14 s");
        assert_eq!(super::took_label(Duration::from_secs(125)), "2 min 05 s");
    }
}
