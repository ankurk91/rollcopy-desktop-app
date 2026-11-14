//! Main window (PLAN §6): the `MainView` state, its layout and keyboard
//! handling. The parts live in their own files:
//!
//! - `toolbar.rs`, `sidebar.rs` (device card, library tree), `guide.rs`
//!   (connection guide), `grid.rs` / `list.rs` (centre pane),
//!   `preview.rs`, `viewer.rs`, `transfers.rs` (bottom panel)
//! - `events.rs` (worker, queue and tray events), `filters.rs` (chips and
//!   the visible items), `lifecycle.rs` (close and quit), `widgets.rs`

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_device::Phase;
use rollcopy_library::Library;

use crate::prefs::Prefs;
use crate::theme::{self};
use crate::transfers::Transfers;
use crate::tree::{LibraryTree, Selection};
use crate::worker::{CachePaths, Command, Worker};

// Moved out of this file; re-exported so the other panes keep their paths.
pub(crate) use crate::filters::{Filter, SyncFilter};
pub(crate) use crate::guide::{guide, guide_busy, render_guide};
pub(crate) use crate::widgets::{btn, muted, section_label};

pub(crate) const LEFT_W: f32 = 270.;
/// Minimum preview pane width; the grid always keeps room for at least one
/// column. (Default width: `UiState::preview_width`.)
pub(crate) const PREVIEW_MIN_W: f32 = 220.;
pub(crate) const GRID_MIN_W: f32 = 200.;

actions!(
    rollcopy,
    [
        SelectAll,
        ClearSelection,
        MoveLeft,
        MoveRight,
        MoveUp,
        MoveDown,
        ExtendLeft,
        ExtendRight,
        ExtendUp,
        ExtendDown,
        OpenItem,
        OpenPreferences,
        CheckForUpdates,
        ViewOnGitHub,
        ReportIssue,
        ReloadLibrary,
        FocusSearch,
        CloseWindow,
        ShowAbout,
        ShowShortcuts,
        Quit,
    ]
);

/// Drag payload for the preview pane's resize handle.
#[derive(Clone)]
pub(crate) struct ResizePreview;

impl Render for ResizePreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Key context for the main window's bindings.
pub(crate) const KEY_CONTEXT: &str = "Rollcopy";

/// The grid's own keys don't apply while typing in a text box.
const GRID_KEYS: &str = "Rollcopy && !Input";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-f", FocusSearch, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(GRID_KEYS)),
        KeyBinding::new("escape", ClearSelection, Some(GRID_KEYS)),
        KeyBinding::new("left", MoveLeft, Some(GRID_KEYS)),
        KeyBinding::new("right", MoveRight, Some(GRID_KEYS)),
        KeyBinding::new("up", MoveUp, Some(GRID_KEYS)),
        KeyBinding::new("down", MoveDown, Some(GRID_KEYS)),
        KeyBinding::new("shift-left", ExtendLeft, Some(GRID_KEYS)),
        KeyBinding::new("shift-right", ExtendRight, Some(GRID_KEYS)),
        KeyBinding::new("shift-up", ExtendUp, Some(GRID_KEYS)),
        KeyBinding::new("shift-down", ExtendDown, Some(GRID_KEYS)),
        KeyBinding::new("enter", OpenItem, Some(GRID_KEYS)),
        KeyBinding::new("space", OpenItem, Some(GRID_KEYS)),
        // App-wide (PLAN §6 ☰ menu).
        KeyBinding::new("ctrl-,", OpenPreferences, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("f1", ShowShortcuts, None),
        KeyBinding::new("f5", ReloadLibrary, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-r", ReloadLibrary, Some(KEY_CONTEXT)),
    ]);
}

pub(crate) enum Thumb {
    Pending,
    Ready(PathBuf),
    Missing,
}

pub(crate) enum LibState {
    None,
    Loading,
    Loaded { library: Library, tree: LibraryTree },
    Failed(String),
}

pub struct MainView {
    pub(crate) paths: CachePaths,
    pub(crate) commands: async_channel::Sender<Command>,
    pub(crate) focus: FocusHandle,
    pub(crate) phase: Phase,
    pub(crate) library: LibState,
    /// Left-pane tree selection.
    pub(crate) selection: Selection,
    pub(crate) filter: Filter,
    pub(crate) expanded: HashSet<i32>,
    pub(crate) show_details: bool,
    /// Asset indices shown in the grid, newest first.
    pub(crate) visible: Vec<usize>,
    /// Selected items, by device path.
    pub(crate) selected: HashSet<String>,
    /// Grid position that Shift+click extends from.
    pub(crate) anchor: Option<usize>,
    /// Asset index shown in the preview pane.
    pub(crate) focused: Option<usize>,
    pub(crate) thumbs: HashMap<String, Thumb>,
    /// Full-window viewer showing the focused item.
    pub(crate) viewer_open: bool,
    /// Full-size photos for the viewer, by device path.
    pub(crate) full: HashMap<String, crate::viewer::Full>,
    /// Hidden-album items the user chose to see this session, by device
    /// path. Others show blurred.
    pub(crate) revealed: HashSet<String>,
    /// List view (no thumbnails) instead of the grid.
    pub(crate) list_view: bool,
    /// The Albums section of the tree is open.
    pub(crate) albums_open: bool,
    /// List column widths (see `columns::LIBRARY`).
    pub(crate) list_widths: Vec<f32>,
    /// Width of a list row this frame, for fitting the columns.
    pub(crate) list_row_w: f32,
    /// Copy settings the copied state in `in_dest` was worked out for.
    pub(crate) copied_for: Option<rollcopy_queue::Settings>,
    /// Bumped per copied-state scan; older scans' results are dropped.
    pub(crate) copied_gen: u64,
    /// Pixel sizes of images shown in the viewer, read once per file.
    pub(crate) image_dims: HashMap<PathBuf, Option<(u32, u32)>>,
    /// Items per filter chip for the current tree selection, cached so the
    /// header doesn't walk the whole library every frame.
    pub(crate) filter_counts: [usize; 4],
    pub(crate) sync_filter: SyncFilter,
    /// All / copied / not copied, for the current tree selection and filter.
    pub(crate) sync_counts: [usize; 3],
    /// Counts need rebuilding (a copy finished since the last count).
    pub(crate) counts_dirty: bool,
    /// Sort by date taken, newest first (default) or oldest first.
    pub(crate) newest_first: bool,
    /// Search box above the grid, and what's typed in it.
    pub(crate) search: Entity<gpui_kit::component::input::InputState>,
    pub(crate) query: String,
    pub(crate) grid_scroll: UniformListScrollHandle,
    /// Tiles per row at the last render; arrow keys move by this much.
    pub(crate) cols: usize,
    /// Tile size at the last render (tiles stretch to fill each row).
    pub(crate) tile: f32,
    pub(crate) preview_w: f32,
    /// One-line status under the preview ("Opening IMG_0042.MOV…").
    pub(crate) status: Option<String>,
    /// A Reload is running; the current list stays until the new one comes.
    pub(crate) reloading: bool,
    pub(crate) transfers: Transfers,
    /// Copy of the saved preferences, refreshed when they change.
    pub(crate) config: crate::config::Config,
    /// Where an item's copy is in the destination (the ✓ badge), by device
    /// path; `None` if it isn't copied. Filled for on-screen tiles.
    pub(crate) in_dest: HashMap<String, Option<PathBuf>>,
    /// Preview pane shown; opens when an item is selected, × closes it.
    pub(crate) preview_open: bool,
    /// "Copies are still running" prompt on window close.
    pub(crate) quit_prompt: bool,
    pub(crate) about_open: bool,
    pub(crate) shortcuts_open: bool,
    pub(crate) credits_open: bool,
    pub(crate) about_note: Option<String>,
    pub(crate) update_ui: Option<crate::about::UpdateUi>,
    /// (version, release page) found by the startup check.
    pub(crate) update_banner: Option<(String, String)>,
    pub(crate) tray_tx: async_channel::Sender<crate::tray::TrayState>,
    /// A tray host accepted the icon (closing can then hide to the tray).
    pub(crate) tray_available: bool,
    pub(crate) last_tray: Option<crate::tray::TrayState>,
    /// The window was closed during a copy and is waiting in the tray.
    pub(crate) closed_to_tray: bool,
    /// Copies were queued or running at the last event.
    pub(crate) was_active: bool,
    /// Pause state last reported by notification.
    pub(crate) notified_pause: Option<String>,
    pub(crate) _tasks: Vec<Task<()>>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(
        worker: Worker,
        paths: CachePaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = worker.events;
        let tray_tx = worker.tray;
        // `spawn_in` so event handling can reach the window (tray "Show").
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = events.recv().await {
                // Take everything already waiting and redraw once: copies
                // send many progress events (audit PERF-04).
                let mut batch = vec![first];
                while batch.len() < 512
                    && let Ok(ev) = events.try_recv()
                {
                    batch.push(ev);
                }
                let alive = this.update_in(cx, |view, window, cx| {
                    for ev in batch {
                        view.on_event(ev, window, cx);
                    }
                    view.after_event(window, cx);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            theme::apply(this.config.app.theme.into(), window, cx);
            this.sync_tray(cx);
        });
        // Preferences changed (here or in the Preferences window): apply them.
        let search = cx.new(|cx| {
            gpui_kit::component::input::InputState::new(window, cx)
                .placeholder("Search name, place, album, app")
        });
        let searched = cx.subscribe_in(
            &search,
            window,
            |this, state, ev: &gpui_kit::component::input::InputEvent, _, cx| {
                if let gpui_kit::component::input::InputEvent::Change = ev {
                    let q = state.read(cx).value().to_string();
                    if q.trim() != this.query.trim() {
                        this.query = q;
                        this.counts_dirty = true;
                        this.recompute_visible();
                        cx.notify();
                    }
                }
            },
        );
        let prefs = cx.observe_global_in::<Prefs>(window, |this, window, cx| {
            let new = Prefs::get(cx).config.clone();
            if new.app.theme != this.config.app.theme {
                theme::apply(new.app.theme.into(), window, cx);
                this.sync_tray(cx);
            }
            if !new.library.show_recently_deleted && this.selection == Selection::RecentlyDeleted {
                this.selection = Selection::All;
            }
            let hidden_changed = new.library.show_hidden != this.config.library.show_hidden;
            if !new.library.show_hidden && this.selection == Selection::Hidden {
                this.selection = Selection::All;
            }
            this.config = new;
            if hidden_changed {
                this.rebuild_tree(cx);
            }
            this.sync_queue_settings(cx);
            cx.notify();
        });
        // Ask before closing while copies are running (PLAN §6 tray rule; the
        // tray itself comes in M6). Window state is saved on the way out.
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            weak.update(cx, |view, cx| view.confirm_close(window, cx))
                .unwrap_or(true)
        });
        let saved_ui = Prefs::get(cx).ui.clone();
        let config = Prefs::get(cx).config.clone();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            paths,
            commands: worker.commands,
            focus,
            phase: Phase::NoDevice { hint: None },
            library: LibState::None,
            selection: Selection::All,
            filter: Filter::All,
            expanded: HashSet::new(),
            show_details: false,
            visible: Vec::new(),
            selected: HashSet::new(),
            anchor: None,
            focused: None,
            thumbs: HashMap::new(),
            viewer_open: false,
            full: HashMap::new(),
            revealed: HashSet::new(),
            list_view: saved_ui.list_view,
            albums_open: false,
            list_widths: crate::columns::LIBRARY.widths_from(&saved_ui.list_columns),
            list_row_w: 800.,
            copied_for: None,
            copied_gen: 0,
            image_dims: HashMap::new(),
            filter_counts: [0; 4],
            sync_filter: SyncFilter::All,
            sync_counts: [0; 3],
            counts_dirty: false,
            newest_first: true,
            search,
            query: String::new(),
            grid_scroll: UniformListScrollHandle::new(),
            cols: 1,
            tile: crate::grid::MIN_TILE,
            preview_w: saved_ui.preview_width.max(PREVIEW_MIN_W),
            status: None,
            reloading: false,
            transfers: Transfers {
                expanded: saved_ui.transfers_expanded,
                widths: crate::columns::TRANSFERS.widths_from(&saved_ui.transfer_columns),
                height: saved_ui.transfers_height.max(crate::transfers::PANEL_MIN_H),
                row_w: 800.,
                ..Transfers::default()
            },
            config,
            in_dest: HashMap::new(),
            preview_open: false,
            quit_prompt: false,
            about_open: false,
            shortcuts_open: false,
            credits_open: false,
            about_note: None,
            update_ui: None,
            update_banner: None,
            tray_tx,
            tray_available: false,
            last_tray: None,
            closed_to_tray: false,
            was_active: false,
            notified_pause: None,
            _tasks: vec![task],
            _subscriptions: vec![appearance, prefs, searched],
        };
        this.maybe_check_on_startup(cx);
        this
    }

    pub(crate) fn assets(&self) -> &[rollcopy_library::Asset] {
        match &self.library {
            LibState::Loaded { library, .. } => &library.assets,
            _ => &[],
        }
    }

    pub(crate) fn udid(&self) -> Option<&str> {
        match &self.phase {
            Phase::Ready { info } => Some(&info.udid),
            _ => None,
        }
    }

    /// A Hidden-album item the user hasn't chosen to see: shown blurred,
    /// and its full-size image isn't fetched.
    pub(crate) fn veiled(&self, a: &rollcopy_library::Asset) -> bool {
        a.hidden && !self.revealed.contains(&a.device_path())
    }

    /// Thumbnail to show for an item: the blurred copy while it's veiled.
    pub(crate) fn thumb_for(&self, a: &rollcopy_library::Asset) -> Option<PathBuf> {
        let Some(Thumb::Ready(path)) = self.thumbs.get(&a.device_path()) else {
            return None;
        };
        if a.hidden && !self.veiled(a) {
            // The map holds the blurred copy; the real one sits beside it.
            return self.udid().map(|u| self.paths.thumb(u, a));
        }
        Some(path.clone())
    }

    /// Shows or blurs a Hidden-album item again.
    pub(crate) fn set_revealed(&mut self, key: String, show: bool) {
        if show {
            self.revealed.insert(key);
        } else {
            self.revealed.remove(&key);
        }
    }

    pub(crate) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all_visible();
        cx.notify();
    }

    pub(crate) fn select_all_visible(&mut self) {
        let keys: Vec<String> = self
            .visible
            .iter()
            .map(|&i| self.assets()[i].device_path())
            .collect();
        self.selected.extend(keys);
    }

    pub(crate) fn clear_selection(
        &mut self,
        _: &ClearSelection,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Esc closes the viewer first.
        if self.viewer_open {
            self.viewer_open = false;
            cx.notify();
            return;
        }
        self.selected.clear();
        self.anchor = None;
        cx.notify();
    }

    pub(crate) fn key_move(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        self.move_cursor(delta, extend);
        cx.notify();
    }

    pub(crate) fn row_delta(&self) -> isize {
        self.cols.max(1) as isize
    }

    pub(crate) fn resize_preview(
        &mut self,
        ev: &DragMoveEvent<ResizePreview>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let width = window.viewport_size().width / px(1.);
        let x = ev.event.position.x / px(1.);
        let max = (width - LEFT_W - GRID_MIN_W).max(PREVIEW_MIN_W);
        self.preview_w = (width - x).clamp(PREVIEW_MIN_W, max);
        cx.notify();
    }

    pub(crate) fn select(&mut self, sel: Selection) {
        if let Selection::Year(y) = sel
            && self.selection == sel
            && !self.expanded.remove(&y)
        {
            self.expanded.insert(y);
        } else if let Selection::Year(y) | Selection::Month(y, _) = sel {
            self.expanded.insert(y);
        }
        self.selection = sel;
        self.recompute_visible();
    }

    // ---- toolbar -------------------------------------------------------

    // ---- left pane ---------------------------------------------------

    // ---- centre pane -------------------------------------------------

    pub(crate) fn render_center(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match (&self.phase, &self.library) {
            (Phase::Ready { .. }, LibState::Loaded { .. }) => self.render_grid(window, cx),
            (Phase::Ready { .. }, LibState::Failed(e)) => guide(
                IconName::TriangleAlert,
                "Couldn't read the Photos library",
                e.clone(),
                true,
                cx,
            ),
            (Phase::Ready { .. }, _) => guide_busy(
                IconName::Images,
                "Reading your Photos library…",
                "This takes a few seconds.",
                cx,
            ),
            (phase, _) => render_guide(phase, cx),
        }
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let center = self.render_center(window, cx);
        let transfers = self.render_transfers(window, cx);
        let conflict = self.render_conflict(cx);
        let toolbar = self.render_toolbar(window, cx);
        // The preview pane and viewer need the focused item's thumbnail even
        // when the grid didn't ask for it (list view loads none).
        if (self.preview_open || self.viewer_open)
            && let Some(idx) = self.focused
        {
            self.request_thumbs_for(&[idx]);
        }
        let viewer = self.render_viewer(window, cx);
        let ready = matches!(self.library, LibState::Loaded { .. })
            && matches!(self.phase, Phase::Ready { .. });
        // Resize edges and shadow when Rollcopy draws its own frame; a plain
        // wrapper otherwise.
        crate::widgets::window_frame().child(
            v_flex()
                .track_focus(&self.focus)
                .key_context(KEY_CONTEXT)
                .on_action(cx.listener(Self::select_all))
                .on_action(cx.listener(Self::clear_selection))
                .on_action(cx.listener(|this, _: &MoveLeft, _, cx| this.key_move(-1, false, cx)))
                .on_action(cx.listener(|this, _: &MoveRight, _, cx| this.key_move(1, false, cx)))
                .on_action(cx.listener(|this, _: &MoveUp, _, cx| {
                    if this.viewer_open {
                        return;
                    }
                    let d = this.row_delta();
                    this.key_move(-d, false, cx)
                }))
                .on_action(cx.listener(|this, _: &MoveDown, _, cx| {
                    if this.viewer_open {
                        return;
                    }
                    let d = this.row_delta();
                    this.key_move(d, false, cx)
                }))
                .on_action(cx.listener(|this, _: &ExtendLeft, _, cx| this.key_move(-1, true, cx)))
                .on_action(cx.listener(|this, _: &ExtendRight, _, cx| this.key_move(1, true, cx)))
                .on_action(cx.listener(|this, _: &ExtendUp, _, cx| {
                    if this.viewer_open {
                        return;
                    }
                    let d = this.row_delta();
                    this.key_move(-d, true, cx)
                }))
                .on_action(cx.listener(|this, _: &ExtendDown, _, cx| {
                    if this.viewer_open {
                        return;
                    }
                    let d = this.row_delta();
                    this.key_move(d, true, cx)
                }))
                .on_action(cx.listener(|this, _: &OpenItem, _, cx| {
                    this.activate_focused();
                    cx.notify();
                }))
                .on_drag_move(cx.listener(Self::resize_preview))
                .on_drag_move(cx.listener(Self::resize_transfers))
                .on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
                .on_action(cx.listener(|this, _: &ReportIssue, _, cx| this.report_issue(cx)))
                // Our own title bar's ×: same checks as the window manager's.
                .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                    if this.confirm_close(window, cx) {
                        window.remove_window();
                    }
                }))
                .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                    this.search.update(cx, |s, cx| s.focus(window, cx));
                }))
                .on_action(cx.listener(|this, _: &ReloadLibrary, _, cx| {
                    this.reload_library();
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ShowShortcuts, _, cx| {
                    this.shortcuts_open = true;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &ShowAbout, _, cx| {
                    this.about_open = true;
                    this.about_note = None;
                    cx.notify();
                }))
                .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| {
                    this.check_for_updates(true, cx);
                    cx.notify();
                }))
                .size_full()
                .relative()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(toolbar)
                .children(self.render_update_banner(cx))
                .children(Prefs::get(cx).notice.clone().map(|n| {
                    div()
                        .w_full()
                        .px_4()
                        .py_1()
                        .text_sm()
                        .bg(theme.warning.opacity(0.15))
                        .child(n)
                }))
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(self.render_left(cx))
                        .child(div().flex_1().h_full().min_w_0().child(center))
                        .when(ready && self.preview_open, |d| {
                            d.child(self.render_preview(cx))
                        }),
                )
                .children(transfers)
                .children(conflict)
                .children(viewer)
                .children(self.render_about(cx))
                .children(self.render_shortcuts(cx))
                .children(self.render_update_dialog(cx))
                .children(self.render_quit_prompt(cx)),
        )
    }
}

// ---- connection guide (PLAN §5) ---------------------------------------

// ---- small helpers ------------------------------------------------------
