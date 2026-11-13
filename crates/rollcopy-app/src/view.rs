//! Main window (PLAN §6): toolbar, left pane with device card and library
//! tree, centre pane with the connection guide or the thumbnail grid
//! (`grid.rs`), right pane with preview and info (`preview.rs`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_device::{DeviceInfo, NoDeviceHint, Phase};
use rollcopy_library::{Library, MediaKind};

use crate::prefs::Prefs;
use crate::theme::{self, ThemeChoice};
use crate::transfers::Transfers;
use crate::tree::{LibraryTree, Selection, month_name};
use crate::worker::{CachePaths, Command, Event, Worker};

const INSTALL_USBMUXD: &str = "sudo apt install usbmuxd";
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

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-a", SelectAll, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", ClearSelection, Some(KEY_CONTEXT)),
        KeyBinding::new("left", MoveLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("right", MoveRight, Some(KEY_CONTEXT)),
        KeyBinding::new("up", MoveUp, Some(KEY_CONTEXT)),
        KeyBinding::new("down", MoveDown, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-left", ExtendLeft, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-right", ExtendRight, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-up", ExtendUp, Some(KEY_CONTEXT)),
        KeyBinding::new("shift-down", ExtendDown, Some(KEY_CONTEXT)),
        KeyBinding::new("enter", OpenItem, Some(KEY_CONTEXT)),
        KeyBinding::new("space", OpenItem, Some(KEY_CONTEXT)),
        // App-wide (PLAN §6 ☰ menu).
        KeyBinding::new("ctrl-,", OpenPreferences, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("f1", ShowShortcuts, None),
        KeyBinding::new("f5", ReloadLibrary, Some(KEY_CONTEXT)),
        KeyBinding::new("ctrl-r", ReloadLibrary, Some(KEY_CONTEXT)),
    ]);
}

/// Centre-pane filter chips (PLAN §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    #[default]
    All,
    Photos,
    Videos,
    Edited,
}

impl Filter {
    pub(crate) const ALL: [Filter; 4] =
        [Filter::All, Filter::Photos, Filter::Videos, Filter::Edited];

    pub(crate) fn count_index(self) -> usize {
        self as usize
    }

    pub(crate) fn matches(self, a: &rollcopy_library::Asset) -> bool {
        match self {
            Filter::All => true,
            Filter::Photos => a.kind == MediaKind::Photo,
            Filter::Videos => a.kind == MediaKind::Video,
            Filter::Edited => a.is_edited(),
        }
    }
}

/// Copied-state chips: is the item already in the Save-to folder?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SyncFilter {
    #[default]
    All,
    Copied,
    NotCopied,
}

impl SyncFilter {
    pub(crate) fn matches(self, copied: bool) -> bool {
        match self {
            SyncFilter::All => true,
            SyncFilter::Copied => copied,
            SyncFilter::NotCopied => !copied,
        }
    }
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
    expanded: HashSet<i32>,
    show_details: bool,
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
    tray_tx: async_channel::Sender<crate::tray::TrayState>,
    /// A tray host accepted the icon (closing can then hide to the tray).
    tray_available: bool,
    last_tray: Option<crate::tray::TrayState>,
    /// The window was closed during a copy and is waiting in the tray.
    closed_to_tray: bool,
    /// Copies were queued or running at the last event.
    was_active: bool,
    /// Pause state last reported by notification.
    notified_pause: Option<String>,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
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
                    view.after_event(window);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            theme::apply(this.config.app.theme.into(), window, cx);
        });
        // Preferences changed (here or in the Preferences window): apply them.
        let prefs = cx.observe_global_in::<Prefs>(window, |this, window, cx| {
            let new = Prefs::get(cx).config.clone();
            if new.app.theme != this.config.app.theme {
                theme::apply(new.app.theme.into(), window, cx);
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
            copied_for: None,
            copied_gen: 0,
            image_dims: HashMap::new(),
            filter_counts: [0; 4],
            sync_filter: SyncFilter::All,
            sync_counts: [0; 3],
            counts_dirty: false,
            newest_first: true,
            grid_scroll: UniformListScrollHandle::new(),
            cols: 1,
            tile: crate::grid::MIN_TILE,
            preview_w: saved_ui.preview_width.max(PREVIEW_MIN_W),
            status: None,
            reloading: false,
            transfers: Transfers {
                expanded: saved_ui.transfers_expanded,
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
            _subscriptions: vec![appearance, prefs],
        };
        this.maybe_check_on_startup(cx);
        this
    }

    fn on_event(&mut self, ev: Event, window: &mut Window, cx: &mut Context<Self>) {
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
            Event::Opened { name } => self.status = Some(format!("Opened {name}")),
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

    /// Rebuilds the tree after the Hidden-album preference changed.
    fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        let show_hidden = self.config.library.show_hidden;
        if let LibState::Loaded { library, tree } = &mut self.library {
            *tree = LibraryTree::build(&library.assets, show_hidden);
        }
        self.forget_hidden_thumbs(cx);
        self.counts_dirty = true;
        self.recompute_visible();
    }

    /// With the Hidden album not shown, deletes cached thumbnails of its
    /// items (some may be cached from before, or from when it was shown).
    fn forget_hidden_thumbs(&mut self, cx: &mut Context<Self>) {
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

    /// Rebuilds the grid's item list after the tree selection, filter or
    /// library changed. Clears the multi-selection (it belongs to a view).
    /// Rebuilds the chip counts for the current tree selection. Copied state
    /// comes from [`MainView::refresh_copied`]; items not checked yet count
    /// as not copied.
    pub(crate) fn recount(&mut self) {
        let (sel, filter) = (self.selection, self.filter);
        let show_hidden = self.config.library.show_hidden;
        let LibState::Loaded { library, .. } = &self.library else {
            self.filter_counts = [0; 4];
            self.sync_counts = [0; 3];
            return;
        };
        let mut counts = [0; 4];
        let mut sync = [0; 3];
        for a in library
            .assets
            .iter()
            .filter(|a| LibraryTree::contains(sel, a, show_hidden))
        {
            for (n, f) in counts.iter_mut().zip(Filter::ALL) {
                if f.matches(a) {
                    *n += 1;
                }
            }
            if !filter.matches(a) {
                continue;
            }
            let copied = matches!(self.in_dest.get(&a.device_path()), Some(Some(_)));
            sync[0] += 1;
            sync[if copied { 1 } else { 2 }] += 1;
        }
        self.filter_counts = counts;
        self.sync_counts = sync;
        self.counts_dirty = false;
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

    /// Whether the item with this device path is known to be copied.
    pub(crate) fn is_copied(&self, key: &str) -> bool {
        matches!(self.in_dest.get(key), Some(Some(_)))
    }

    pub(crate) fn recompute_visible(&mut self) {
        self.recount();
        let (sel, filter, sync) = (self.selection, self.filter, self.sync_filter);
        let show_hidden = self.config.library.show_hidden;
        let mut visible: Vec<usize> = self
            .assets()
            .iter()
            .enumerate()
            .filter(|(_, a)| {
                LibraryTree::contains(sel, a, show_hidden)
                    && filter.matches(a)
                    && sync.matches(self.is_copied(&a.device_path()))
            })
            .map(|(i, _)| i)
            .collect();
        // By date taken; undated items last either way.
        let assets = self.assets();
        visible.sort_by(|&i, &j| {
            let (a, b) = (&assets[i], &assets[j]);
            let by_date = match (a.captured, b.captured) {
                (Some(x), Some(y)) if self.newest_first => y.cmp(&x),
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            };
            by_date.then_with(|| a.filename.cmp(&b.filename))
        });
        self.visible = visible;
        self.selected.clear();
        self.anchor = None;
        self.focused = self.visible.first().copied();
        self.grid_scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
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

    fn clear_selection(&mut self, _: &ClearSelection, _: &mut Window, cx: &mut Context<Self>) {
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

    fn key_move(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        self.move_cursor(delta, extend);
        cx.notify();
    }

    fn row_delta(&self) -> isize {
        self.cols.max(1) as isize
    }

    fn resize_preview(
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

    fn select(&mut self, sel: Selection) {
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let save_to = self.render_save_to(cx);
        let status = match &self.phase {
            Phase::Ready { info } => info.name.clone(),
            Phase::NoDevice { .. } | Phase::UsbmuxdMissing => "No iPhone connected".into(),
            _ => "Connecting…".into(),
        };
        let ready = matches!(self.library, LibState::Loaded { .. });
        h_flex()
            .w_full()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(Icon::new(IconName::Smartphone).small())
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Rollcopy"))
            .child(div().w(px(24.)))
            .children(save_to)
            .child(div().flex_1())
            .child(self.render_app_menu())
            .child(
                btn("prefs")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("Preferences (Ctrl+,)")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenPreferences), cx)
                    }),
            )
            .child(
                btn("theme-toggle")
                    .xsmall()
                    .ghost()
                    .icon(match ThemeChoice::from(self.config.app.theme) {
                        ThemeChoice::System => IconName::SunMoon,
                        ThemeChoice::Light => IconName::Sun,
                        ThemeChoice::Dark => IconName::Moon,
                    })
                    .tooltip(ThemeChoice::from(self.config.app.theme).label())
                    .on_click(cx.listener(|this, _, window, cx| {
                        // Saved; the Prefs observer applies it.
                        let next = ThemeChoice::from(this.config.app.theme).next();
                        Prefs::update_config(cx, |c| c.app.theme = next.into());
                        let _ = window;
                        cx.notify();
                    })),
            )
            .child(
                btn("github")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Github)
                    .tooltip("Rollcopy on GitHub")
                    .on_click(|_, _, cx| cx.open_url(crate::update::REPO_URL)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
            .when(ready, |d| {
                d.child(
                    btn("reload")
                        .xsmall()
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .tooltip("Reload from iPhone (F5)")
                        .disabled(self.reloading)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reload_library();
                            cx.notify();
                        })),
                )
            })
    }

    // ---- left pane ---------------------------------------------------

    fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut pane = v_flex()
            .id("left-pane")
            .w(px(LEFT_W))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scrollbar()
            .p_3()
            .gap_3()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(section_label("DEVICES", &theme));

        pane = match &self.phase {
            Phase::Ready { info } => pane.child(self.render_device_card(info, cx)),
            Phase::NoDevice { .. } | Phase::UsbmuxdMissing => pane.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("No iPhone connected"),
            ),
            _ => pane.child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(Spinner::new().small())
                    .child("Connecting…"),
            ),
        };

        if let LibState::Loaded { tree, .. } = &self.library {
            pane = pane
                .child(section_label("LIBRARY", &theme))
                .child(self.render_tree(tree, cx));
        }
        pane
    }

    fn render_device_card(&self, info: &DeviceInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut card = v_flex()
            .gap_1()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Smartphone).small())
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(info.name.clone()),
                    ),
            )
            .child(muted(info.model.clone(), &theme))
            .child(muted(format!("iOS {}", info.ios_version), &theme));

        if let (Some(total), Some(free)) = (info.storage_total, info.storage_free) {
            let used = total.saturating_sub(free);
            let frac = if total > 0 {
                used as f32 / total as f32
            } else {
                0.
            };
            card = card
                .child(
                    div()
                        .mt_1()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme.muted)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .w(relative(frac.clamp(0., 1.)))
                                .bg(theme.primary),
                        ),
                )
                .child(muted(format!("{} used of {}", gb(used), gb(total)), &theme));
        }
        if let Some(pct) = info.battery_percent {
            let charging = info.battery_charging == Some(true);
            card = card.child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(
                        Icon::new(if charging {
                            IconName::BatteryCharging
                        } else {
                            IconName::Battery
                        })
                        .xsmall(),
                    )
                    .child(if charging {
                        format!("{pct}% · charging")
                    } else {
                        format!("{pct}%")
                    }),
            );
        }

        let show = self.show_details;
        card = card.child(
            div()
                .id("device-details")
                .mt_1()
                .text_xs()
                .text_color(theme.link)
                .cursor_pointer()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    Icon::new(if show {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall(),
                )
                .child(if show { "Hide details" } else { "Details" })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_details = !this.show_details;
                    cx.notify();
                })),
        );
        if show {
            let backup = match info.icloud_backup {
                Some(true) => "on",
                Some(false) => "off",
                None => "unknown",
            };
            card = card
                .child(detail(
                    "Serial",
                    info.serial_number.as_deref().unwrap_or("-"),
                    &theme,
                ))
                .child(detail("UDID", &info.udid, &theme))
                .child(detail("iCloud Backup", backup, &theme));
        }
        card
    }

    fn render_tree(&self, tree: &LibraryTree, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().gap_0p5().child(self.tree_row(
            "all".into(),
            None,
            "All media".into(),
            tree.all,
            Selection::All,
            false,
            cx,
        ));
        for y in &tree.years {
            let open = self.expanded.contains(&y.year);
            list = list.child(self.tree_row(
                format!("y{}", y.year).into(),
                Some(open),
                y.year.to_string().into(),
                y.count,
                Selection::Year(y.year),
                false,
                cx,
            ));
            if open {
                for (m, n) in &y.months {
                    list = list.child(div().pl_5().child(self.tree_row(
                        format!("m{}-{m}", y.year).into(),
                        None,
                        month_name(*m).into(),
                        *n,
                        Selection::Month(y.year, *m),
                        false,
                        cx,
                    )));
                }
            }
        }
        if tree.undated > 0 {
            list = list.child(self.tree_row(
                "undated".into(),
                None,
                "Undated".into(),
                tree.undated,
                Selection::Undated,
                false,
                cx,
            ));
        }
        if self.config.library.show_hidden && tree.hidden > 0 {
            list = list.child(self.tree_row(
                "hidden".into(),
                None,
                "Hidden".into(),
                tree.hidden,
                Selection::Hidden,
                false,
                cx,
            ));
        }
        // Greyed out, never copied (PLAN §6); can be hidden in Preferences.
        if self.config.library.show_recently_deleted {
            list = list.child(self.tree_row(
                "deleted".into(),
                None,
                "Recently Deleted".into(),
                tree.recently_deleted,
                Selection::RecentlyDeleted,
                true,
                cx,
            ));
        }
        list
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_row(
        &self,
        id: SharedString,
        expanded: Option<bool>,
        label: SharedString,
        count: usize,
        sel: Selection,
        greyed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected = self.selection == sel;
        h_flex()
            .id(id)
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.accent))
            .when(!selected, |d| d.hover(|s| s.bg(theme.muted)))
            .when(greyed, |d| d.text_color(theme.muted_foreground))
            .child(match (expanded, sel) {
                // The chevron toggles on its own, whatever is selected.
                (Some(open), Selection::Year(year)) => div()
                    .id(SharedString::from(format!("chev-{year}")))
                    .p_0p5()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.muted))
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .xsmall(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&year) {
                            this.expanded.insert(year);
                        }
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .into_any_element(),
                (_, Selection::Hidden) => div()
                    .w(px(16.))
                    .child(Icon::new(IconName::EyeOff).xsmall())
                    .into_any_element(),
                (_, Selection::RecentlyDeleted) => div()
                    .w(px(16.))
                    .child(Icon::new(IconName::Trash).xsmall())
                    .into_any_element(),
                _ => div().w(px(16.)).into_any_element(),
            })
            .child(div().flex_1().child(label))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(count.to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(sel);
                cx.notify();
            }))
    }

    // ---- centre pane -------------------------------------------------

    fn render_center(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
        let transfers = self.render_transfers(cx);
        let conflict = self.render_conflict(cx);
        let toolbar = self.render_toolbar(cx).into_any_element();
        let viewer = self.render_viewer(window, cx);
        let ready = matches!(self.library, LibState::Loaded { .. })
            && matches!(self.phase, Phase::Ready { .. });
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
            .on_action(cx.listener(|this, _: &Quit, window, cx| this.quit(window, cx)))
            .on_action(cx.listener(|this, _: &ReportIssue, _, cx| this.report_issue(cx)))
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
            .children(self.render_quit_prompt(cx))
    }
}

// ---- connection guide (PLAN §5) ---------------------------------------

fn render_guide(phase: &Phase, cx: &mut Context<MainView>) -> AnyElement {
    match phase {
        Phase::UsbmuxdMissing => {
            let theme = cx.theme().clone();
            guide_with(
                IconName::Terminal,
                "Install usbmuxd",
                "Rollcopy needs the usbmuxd service to talk to your iPhone. Install it, then plug your iPhone in.",
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme.muted)
                            .font_family("monospace")
                            .text_sm()
                            .child(INSTALL_USBMUXD),
                    )
                    .child(
                        btn("copy-install")
                            .small()
                            .outline()
                            .icon(IconName::Copy)
                            .label("Copy")
                            .on_click(|_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    INSTALL_USBMUXD.to_owned(),
                                ))
                            }),
                    )
                    .into_any_element(),
                cx,
            )
        }
        Phase::NoDevice { hint: None } => guide(
            IconName::Plug,
            "Plug in your iPhone with a cable",
            "Use a cable that carries data, connected straight to this computer.".into(),
            false,
            cx,
        ),
        Phase::NoDevice {
            hint: Some(NoDeviceHint::Flaky),
        } => guide(
            IconName::Unplug,
            "The connection keeps dropping",
            "Your iPhone connected for a moment and then disconnected. Try another cable or another USB port, without a hub."
                .into(),
            true,
            cx,
        ),
        Phase::NoDevice {
            hint: Some(NoDeviceHint::AppleUsbNotListed),
        } => guide(
            IconName::Usb,
            "Your iPhone isn't responding",
            "It's plugged in but not answering. Unplug it and plug it back in, or try another cable or USB port."
                .into(),
            true,
            cx,
        ),
        Phase::Connecting { .. } => guide_busy(
            IconName::Smartphone,
            "Connecting to your iPhone…",
            "",
            cx,
        ),
        Phase::WaitingForTrust { .. } => guide_busy(
            IconName::ShieldCheck,
            "Unlock your iPhone and tap Trust",
            "Then enter your passcode. This lets Rollcopy read your photos and videos.",
            cx,
        ),
        Phase::TrustDenied { .. } => guide(
            IconName::ShieldX,
            "Your iPhone said Don't Trust",
            "Unplug your iPhone and plug it back in to be asked again.".into(),
            true,
            cx,
        ),
        Phase::Locked { .. } => guide_busy(
            IconName::Lock,
            "Unlock your iPhone",
            "Rollcopy continues as soon as it's unlocked.",
            cx,
        ),
        Phase::Failed { message, .. } => guide_busy(
            IconName::TriangleAlert,
            "Couldn't connect to your iPhone",
            message,
            cx,
        ),
        // Ready is handled by the caller.
        Phase::Ready { .. } => div().into_any_element(),
    }
}

fn guide(
    icon: IconName,
    title: &str,
    body: String,
    warn: bool,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    guide_frame(
        Icon::new(icon)
            .with_size(px(48.))
            .text_color(if warn {
                theme.warning
            } else {
                theme.muted_foreground
            })
            .into_any_element(),
        title,
        &body,
        None,
        cx,
    )
}

fn guide_busy(icon: IconName, title: &str, body: &str, cx: &mut Context<MainView>) -> AnyElement {
    let theme = cx.theme().clone();
    guide_frame(
        Icon::new(icon)
            .with_size(px(48.))
            .text_color(theme.muted_foreground)
            .into_any_element(),
        title,
        body,
        Some(Spinner::new().into_any_element()),
        cx,
    )
}

fn guide_with(
    icon: IconName,
    title: &str,
    body: &str,
    extra: AnyElement,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    guide_frame(
        Icon::new(icon)
            .with_size(px(48.))
            .text_color(theme.muted_foreground)
            .into_any_element(),
        title,
        body,
        Some(extra),
        cx,
    )
}

fn guide_frame(
    icon: AnyElement,
    title: &str,
    body: &str,
    extra: Option<AnyElement>,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_8()
        .child(icon)
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .when(!body.is_empty(), |d| {
            d.child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(body.to_owned()),
            )
        })
        .children(extra)
        .into_any_element()
}

// ---- small helpers ------------------------------------------------------

pub(crate) fn section_label(text: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(text)
}

pub(crate) fn muted(text: String, theme: &Theme) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(text)
}

fn detail(label: &str, value: &str, theme: &Theme) -> impl IntoElement {
    v_flex()
        .text_xs()
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child(label.to_owned()),
        )
        .child(div().child(value.to_owned()))
}

pub(crate) fn gb(bytes: u64) -> String {
    format!("{:.0} GB", bytes as f64 / 1e9)
}

/// Development aid: `ROLLCOPY_DEV_REPEAT=N` repeats the library N times to
/// test grid performance at PLAN §6's 5,000 items with a small phone.
/// Repeated items share device paths, so selection treats them as one.
/// Debug builds only (audit MNT-09).
fn dev_repeat(library: &mut Library) {
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
fn phase_label(p: &Phase) -> &'static str {
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

/// A button with the hand cursor (gpui-component defaults to the arrow).
pub(crate) fn btn(id: impl Into<ElementId>) -> Button {
    Button::new(id).cursor_pointer()
}

impl MainView {
    /// Number of items still queued, copying or waiting for an answer.
    pub(crate) fn unfinished_transfers(&self) -> usize {
        self.transfers
            .rows
            .iter()
            .filter(|r| !r.state.is_finished())
            .count()
    }

    /// Saves window size/position, preview width and queue panel state.
    pub(crate) fn save_ui_state(&self, window: &Window, cx: &mut App) {
        let bounds = window.window_bounds();
        let maximized = matches!(
            bounds,
            WindowBounds::Maximized(_) | WindowBounds::Fullscreen(_)
        );
        let b = bounds.get_bounds();
        let saved = crate::config::SavedBounds {
            x: b.origin.x / px(1.),
            y: b.origin.y / px(1.),
            width: b.size.width / px(1.),
            height: b.size.height / px(1.),
            maximized,
        };
        let (preview, expanded) = (self.preview_w, self.transfers.expanded);
        let list_view = self.list_view;
        Prefs::update_ui(cx, |ui| {
            ui.window = Some(saved);
            ui.preview_width = preview;
            ui.transfers_expanded = expanded;
            ui.list_view = list_view;
        });
    }

    /// Quit after saving window state.
    pub(crate) fn quit(&self, window: &Window, cx: &mut App) {
        self.save_ui_state(window, cx);
        cx.quit();
    }

    /// Window close requested: allow it, hide to the tray while copying, or
    /// show the "still copying" prompt when there's no tray.
    fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.unfinished_transfers() == 0 {
            self.save_ui_state(window, cx);
            return true;
        }
        if self.tray_available && self.config.app.keep_in_tray {
            self.closed_to_tray = true;
            self.save_ui_state(window, cx);
            window.minimize_window();
            crate::notify::send(
                "Rollcopy is still copying",
                "It keeps going in the tray. Click the tray icon to bring it back.",
            );
            return false;
        }
        self.quit_prompt = true;
        cx.notify();
        false
    }

    fn render_quit_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.quit_prompt {
            return None;
        }
        let theme = cx.theme().clone();
        let left = self.unfinished_transfers();
        let card = v_flex()
            .w(px(440.))
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
                    .child("Copies are still running"),
            )
            .child(muted(
                format!(
                    "{left} item(s) haven't been copied yet. If you quit now they are not copied; \
                     files already copied are kept, and nothing is left half-written."
                ),
                &theme,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(btn("quit-cancel").outline().label("Keep copying").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.quit_prompt = false;
                            cx.notify();
                        }),
                    ))
                    .child(
                        btn("quit-anyway")
                            .danger()
                            .label("Quit")
                            .on_click(cx.listener(|this, _, window, cx| this.quit(window, cx))),
                    ),
            );
        Some(
            div()
                .id("quit-backdrop")
                .absolute()
                .inset_0()
                .bg(hsla(0., 0., 0., 0.45))
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(card)
                .into_any_element(),
        )
    }
}

impl MainView {
    /// Runs after every worker event: tray state, finished-batch handling.
    fn after_event(&mut self, window: &Window) {
        self.sync_tray();
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
        if self.was_active && !active {
            self.on_batch_finished(window);
        }
        self.was_active = active;
    }

    fn on_batch_finished(&mut self, window: &Window) {
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
            if let Some(dest) = self.dest_root() {
                body.push_str(&format!("\n{}", dest.display()));
            }
            crate::notify::send("Transfers finished", body);
        }
        // Closed during the copy: Rollcopy stays in the tray (decided
        // 2026-11-12). Quitting here used to cut off the notification.
    }

    fn on_tray_action(
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
    fn sync_tray(&mut self) {
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
                None => format!("Copying {done} of {}{current}", rows.len()),
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
        };
        if self.last_tray.as_ref() != Some(&state) {
            let _ = self.tray_tx.try_send(state.clone());
            self.last_tray = Some(state);
        }
    }
}
