//! Centre pane: the virtualised thumbnail grid (PLAN §6); the bar above
//! it is in `library_bar.rs`. Only rows on screen are built, and only their thumbnails are
//! requested, so a 5,000-item library scrolls without loading everything.

use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::MediaKind;

use crate::tree::{Selection, month_name};
use crate::view::{LEFT_W, LibState, MainView, Thumb, btn};
use crate::worker::{Command, ThumbReq, phone_thumb_path};

/// Thumbnail size the user picks (zoom): the smallest a tile may be; tiles
/// stretch so each row fills the width. The phone's thumbnails are
/// 480×360, so tiles past ~300 px would only get blurrier.
pub(crate) const TILE_DEFAULT: f32 = 160.;
pub(crate) const TILE_MIN: f32 = 80.;
pub(crate) const TILE_MAX: f32 = 300.;
/// Each zoom step changes the size by this factor.
const ZOOM_FACTOR: f32 = 1.2;
/// Touchpad scrolling (pixel deltas) per zoom step.
const ZOOM_PIXELS: f32 = 60.;

/// Columns and tile size for `avail` pixels of row width and a tile size
/// of at least `target`.
pub(crate) fn layout(avail: f32, target: f32) -> (usize, f32) {
    let avail = avail.max(target);
    let cols = (((avail + GAP) / (target + GAP)).floor() as usize).max(1);
    let tile = ((avail - GAP * (cols as f32 - 1.)) / cols as f32).floor();
    (cols, tile)
}

/// The tile size `steps` zoom steps from `current` (positive = bigger).
pub(crate) fn zoomed(current: f32, steps: i32) -> f32 {
    (current * ZOOM_FACTOR.powi(steps))
        .round()
        .clamp(TILE_MIN, TILE_MAX)
}
const GAP: f32 = 8.;
/// Height of a month heading row.
const HEADER_H: f32 = 44.;
/// Side padding of the grid. The scrollbar (16 px) draws over the right
/// one, so the last column never sits under it and no extra room is kept.
pub(crate) const PAD: f32 = 16.;

impl MainView {
    /// Tiles per row for the current window width.
    /// Tiles per row and tile size for the current window. The grid takes
    /// the preview pane's room when the pane is closed.
    fn grid_layout(&self, window: &Window) -> (usize, f32) {
        let width = window.viewport_size().width / px(1.);
        let preview = if self.preview_open {
            self.preview_w
        } else {
            0.
        };
        layout(width - LEFT_W - preview - 2. * PAD, self.tile_target)
    }

    /// Changes the thumbnail size by `steps` (0 = back to the default),
    /// keeps the focused item in view, and remembers the size.
    pub(crate) fn zoom(&mut self, steps: i32, cx: &mut Context<Self>) {
        let next = if steps == 0 {
            TILE_DEFAULT
        } else {
            zoomed(self.tile_target, steps)
        };
        if next == self.tile_target {
            return;
        }
        self.tile_target = next;
        crate::prefs::Prefs::update_ui(cx, |ui| ui.tile_size = next);
        self.scroll_focused_into_view_later = true;
        cx.notify();
    }

    /// Ctrl+wheel over the grid zooms. Registered for the capture phase so
    /// the list doesn't scroll at the same time.
    fn zoom_wheel_listener(cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                window.on_mouse_event(move |ev: &ScrollEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture
                        || !ev.modifiers.control
                        || !bounds.contains(&ev.position)
                    {
                        return;
                    }
                    cx.stop_propagation();
                    let _ = view.update(cx, |this, cx| {
                        let steps = match ev.delta {
                            ScrollDelta::Lines(p) => p.y.signum() as i32,
                            ScrollDelta::Pixels(p) => {
                                this.zoom_pixels += p.y / px(1.);
                                let steps = (this.zoom_pixels / ZOOM_PIXELS).trunc();
                                this.zoom_pixels -= steps * ZOOM_PIXELS;
                                steps as i32
                            }
                        };
                        if steps != 0 {
                            this.zoom(steps, cx);
                        }
                    });
                });
            },
        )
        .absolute()
        .inset_0()
    }

    /// Width of the centre pane's content (between its side paddings).
    fn content_width(&self, window: &Window) -> f32 {
        let width = window.viewport_size().width / px(1.);
        let preview = if self.preview_open {
            self.preview_w
        } else {
            0.
        };
        (width - LEFT_W - preview - 2. * PAD).max(200.)
    }

    pub(crate) fn render_grid(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.counts_dirty {
            self.recount();
        }
        let (cols, tile) = self.grid_layout(window);
        // The list is one item per row; arrow keys move by one either way.
        let cols = if self.list_view { 1 } else { cols };
        self.list_row_w = self.content_width(window);
        self.cols = cols;
        self.tile = tile;
        if !self.list_view {
            self.rebuild_rows(cols);
        }
        if std::mem::take(&mut self.scroll_focused_into_view_later)
            && let Some(pos) = self
                .focused
                .and_then(|i| self.visible.iter().position(|&v| v == i))
        {
            self.reveal(pos, true);
        }

        let header = self.render_grid_header(cx).into_any_element();
        let body = if self.visible.is_empty() {
            self.render_empty(cx)
        } else if self.list_view {
            // No thumbnails: fast, and nothing on screen to see (privacy).
            v_flex()
                .flex_1()
                .min_h_0()
                .child(self.render_list_header(cx))
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(
                            uniform_list(
                                "list",
                                self.visible.len(),
                                cx.processor(move |this, range: Range<usize>, _window, cx| {
                                    range
                                        .map(|pos| this.render_list_row(pos, cx))
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(&self.grid_scroll)
                            .size_full()
                            .px(px(PAD))
                            .pb(px(PAD)),
                        )
                        .vertical_scrollbar(&self.grid_scroll),
                )
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(Self::zoom_wheel_listener(cx))
                .child(
                    list(
                        self.grid_list.clone(),
                        cx.processor(|this, ix: usize, _window, cx| this.render_grid_row(ix, cx)),
                    )
                    .size_full()
                    .px(px(PAD))
                    .pt(px(8.))
                    .pb(px(PAD)),
                )
                .vertical_scrollbar(&self.grid_list)
                .into_any_element()
        };

        v_flex()
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// Queues thumbnails for these asset indices that aren't known yet.
    /// Files already in the local cache are used directly without asking
    /// the phone.
    pub(crate) fn request_thumbs_for(&mut self, indices: &[usize]) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        // Borrow fields, not `self`, so the caches can be updated in the loop.
        let LibState::Loaded { library, .. } = &self.library else {
            return;
        };
        let mut wanted = Vec::new();
        for &idx in indices {
            let Some(a) = library.assets.get(idx) else {
                continue;
            };
            let key = a.device_path();
            if self.thumbs.contains_key(&key) {
                continue;
            }
            let dest = self.paths.thumb(&udid, a);
            // Hidden-album items are shown from a blurred copy first.
            let veil = a.hidden.then(|| self.paths.veil(&udid, a));
            let shown = veil.as_ref().unwrap_or(&dest);
            if shown.exists() {
                self.thumbs.insert(key, Thumb::Ready(shown.clone()));
            } else {
                wanted.push(ThumbReq {
                    key: key.clone(),
                    source: phone_thumb_path(a),
                    dest,
                    veil,
                });
                self.thumbs.insert(key, Thumb::Pending);
            }
        }
        if !wanted.is_empty() {
            let _ = self.commands.try_send(Command::Thumbs {
                udid,
                items: wanted,
            });
        }
    }

    /// Rebuilds the grid's rows when the items, the columns or the grouping
    /// changed. Grouped by month unless a single month is selected.
    fn rebuild_rows(&mut self, cols: usize) {
        let grouped =
            self.sort.groups_by_month() && !matches!(self.selection, Selection::Month(..));
        let key = (self.visible_gen, cols, grouped);
        let Some(old) = self.rows_key.replace(key) else {
            self.build_rows(cols, grouped);
            return;
        };
        if old == key {
            return;
        }
        self.build_rows(cols, grouped);
        // Same items in a new layout (zoom, window size): stay on the
        // focused item instead of jumping to the top.
        if old.0 == key.0 {
            self.scroll_focused_into_view_later = true;
        }
    }

    fn build_rows(&mut self, cols: usize, grouped: bool) {
        let assets = self.assets();
        let months: Vec<_> = self
            .visible
            .iter()
            .map(|&i| assets[i].year_month())
            .collect();
        self.grid_rows = crate::sections::build(&months, cols, grouped);
        self.grid_list
            .reset_with_uniform_height(self.grid_rows.len(), px(self.tile + GAP));
    }

    /// Scrolls so visible position `pos` is on screen; `top` puts its row
    /// (and its month heading) at the top.
    pub(crate) fn reveal(&mut self, pos: usize, top: bool) {
        if self.list_view {
            self.grid_scroll.scroll_to_item(
                pos,
                if top {
                    ScrollStrategy::Center
                } else {
                    ScrollStrategy::Nearest
                },
            );
            return;
        }
        let Some(row) = crate::sections::row_of(&self.grid_rows, pos) else {
            return;
        };
        let header_above =
            row > 0 && matches!(self.grid_rows[row - 1], crate::sections::Row::Header { .. });
        if top {
            let ix = if header_above { row - 1 } else { row };
            self.grid_list.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.),
            });
        } else {
            if header_above {
                self.grid_list.scroll_to_reveal_item(row - 1);
            }
            self.grid_list.scroll_to_reveal_item(row);
        }
    }

    /// One grid row: a month heading or a row of tiles.
    fn render_grid_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.grid_rows.get(ix).cloned() {
            Some(crate::sections::Row::Tiles { start, end }) => {
                let indices: Vec<usize> = self.visible[start..end].to_vec();
                self.request_thumbs_for(&indices);
                h_flex()
                    .h(px(self.tile + GAP))
                    .gap(px(GAP))
                    .items_start()
                    .children((start..end).map(|pos| self.render_tile(pos, cx)))
                    .into_any_element()
            }
            Some(crate::sections::Row::Header { month, start, end }) => {
                self.render_month_header(ix, month, start, end, cx)
            }
            None => div().into_any_element(),
        }
    }

    fn render_month_header(
        &self,
        ix: usize,
        month: Option<(i32, u32)>,
        start: usize,
        end: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let title = match month {
            Some((y, m)) => format!("{} {y}", month_name(m)),
            None => "Undated".to_owned(),
        };
        let n = end - start;
        h_flex()
            .h(px(HEADER_H))
            .pt_3()
            .pb_1()
            .gap_2()
            .items_end()
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .pb(px(1.))
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("{n} item{}", if n == 1 { "" } else { "s" })),
            )
            .child(div().flex_1())
            .child(
                btn(("select-month", ix))
                    .xsmall()
                    .ghost()
                    .label("Select")
                    .tooltip("Add this month to the selection")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let keys: Vec<String> = this.visible[start..end]
                            .iter()
                            .map(|&i| this.assets()[i].device_path())
                            .collect();
                        this.selected.extend(keys);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_tile(&self, pos: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let idx = self.visible[pos];
        let a = &self.assets()[idx];
        let key = a.device_path();
        let selected = self.selected.contains(&key);
        let focused = self.focused == Some(idx);

        let veiled = self.veiled(a);
        let image = match (self.thumbs.get(&key), self.thumb_for(a)) {
            (Some(Thumb::Ready(_)), Some(path)) => img(path)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            (Some(Thumb::Missing), _) => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(if a.kind == MediaKind::Video {
                        IconName::Video
                    } else {
                        IconName::Image
                    })
                    .text_color(theme.muted_foreground),
                )
                .into_any_element(),
            _ => div().size_full().into_any_element(),
        };

        // Marks on the photo: small dark discs or pills, the same in both
        // themes so they read on any picture.
        let scrim = hsla(0., 0., 0., 0.55);
        let disc = |id: (&'static str, usize), icon: IconName, color: Hsla, tip: &'static str| {
            div()
                .id(id)
                .size(px(20.))
                .rounded_full()
                .bg(scrim)
                .flex()
                .items_center()
                .justify_center()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx)
                })
                .child(Icon::new(icon).xsmall().text_color(color))
        };

        div()
            .id(("tile", pos))
            .size(px(self.tile))
            .flex_shrink_0()
            .relative()
            .rounded(px(crate::palette::RADIUS))
            .overflow_hidden()
            .bg(theme.muted)
            .cursor_pointer()
            .border_2()
            .border_color(if selected {
                theme.primary
            } else if focused {
                theme.ring.opacity(0.6)
            } else {
                transparent_black()
            })
            .when(!selected && !focused, |d| {
                d.hover(|s| s.border_color(theme.border))
            })
            .when(a.trashed, |d| d.opacity(0.4))
            .child(image)
            .when(veiled, |d| {
                let key = key.clone();
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            btn(("reveal", pos))
                                .small()
                                .icon(IconName::Eye)
                                .tooltip("Hidden album: click to show")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_revealed(key.clone(), true);
                                    cx.stop_propagation();
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(a.kind == MediaKind::Video, |d| {
                let time = a
                    .duration_secs
                    .map(|s| format!("{}:{:02}", s as u64 / 60, s as u64 % 60));
                d.child(
                    h_flex()
                        .absolute()
                        .bottom_1()
                        .left_1()
                        .h(px(20.))
                        .px_1p5()
                        .gap_1()
                        .items_center()
                        .rounded_full()
                        .bg(scrim)
                        .text_color(white())
                        .text_xs()
                        .child(Icon::new(IconName::Video).xsmall())
                        .children(time),
                )
            })
            .child(
                h_flex()
                    .absolute()
                    .top_1()
                    .right_1()
                    .gap_1()
                    .when(a.is_edited(), |d| {
                        d.child(disc(
                            ("edited", pos),
                            IconName::SlidersHorizontal,
                            white(),
                            "Edited on the iPhone",
                        ))
                    })
                    .when(a.live_photo, |d| {
                        d.child(disc(
                            ("live", pos),
                            IconName::CircleDot,
                            white(),
                            "Live Photo",
                        ))
                    }),
            )
            // Already in the Save-to folder (from what's on disk, PLAN §7.5).
            .when(self.is_copied(&key), |d| {
                d.child(div().absolute().bottom_1().right_1().child(disc(
                    ("copied", pos),
                    IconName::CheckCheck,
                    crate::palette::hsla(crate::palette::COPIED),
                    "Copied: already in the Save to folder",
                )))
            })
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_1()
                        .left_1()
                        .size(px(20.))
                        .rounded_full()
                        .bg(theme.primary)
                        .border_1()
                        .border_color(white())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Check)
                                .xsmall()
                                .text_color(theme.primary_foreground),
                        ),
                )
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.click_tile(pos, ev.modifiers(), ev.click_count());
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Switches between the grid and the list, keeping the focused item in
    /// view, and remembers the choice.
    pub(crate) fn set_list_view(&mut self, list: bool, cx: &mut Context<Self>) {
        if self.list_view == list {
            return;
        }
        self.list_view = list;
        crate::prefs::Prefs::update_ui(cx, |ui| ui.list_view = list);
        // The grid's rows are rebuilt on the next layout; land on the
        // focused item there.
        self.scroll_focused_into_view_later = true;
        cx.notify();
    }

    /// Click, Ctrl+click, Shift+click selection (PLAN §6). A double-click
    /// opens the item in the system viewer.
    pub(crate) fn click_tile(&mut self, pos: usize, mods: Modifiers, clicks: usize) {
        let Some(&idx) = self.visible.get(pos) else {
            return;
        };
        self.focused = Some(idx);
        self.preview_open = true;
        self.status = None;
        if clicks >= 2 {
            self.activate_focused();
            return;
        }
        let key = self.assets()[idx].device_path();
        if mods.shift
            && let Some(anchor) = self.anchor
        {
            let (lo, hi) = (anchor.min(pos), anchor.max(pos));
            if !mods.control {
                self.selected.clear();
            }
            let keys: Vec<String> = self.visible[lo..=hi]
                .iter()
                .map(|&i| self.assets()[i].device_path())
                .collect();
            self.selected.extend(keys);
        } else if mods.control {
            if !self.selected.remove(&key) {
                self.selected.insert(key);
            }
            self.anchor = Some(pos);
        } else {
            self.selected.clear();
            self.selected.insert(key);
            self.anchor = Some(pos);
        }
    }

    /// Arrow-key navigation: moves the focused item by `delta` grid positions
    /// (±1 for left/right, ±columns for up/down), selecting it, or extending
    /// the selection from the anchor when `extend` (Shift) is held. Keeps the
    /// item on screen.
    pub(crate) fn move_cursor(&mut self, delta: isize, extend: bool) {
        let len = self.visible.len();
        if len == 0 {
            return;
        }
        let current = self
            .focused
            .and_then(|i| self.visible.iter().position(|&v| v == i));
        let pos = match current {
            None => 0,
            Some(p) => (p as isize + delta).clamp(0, len as isize - 1) as usize,
        };
        let idx = self.visible[pos];
        self.focused = Some(idx);
        self.preview_open = true;
        self.status = None;
        if extend {
            let anchor = self.anchor.or(current).unwrap_or(pos);
            self.anchor = Some(anchor);
            let (lo, hi) = (anchor.min(pos), anchor.max(pos));
            let keys: Vec<String> = self.visible[lo..=hi]
                .iter()
                .map(|&i| self.assets()[i].device_path())
                .collect();
            self.selected = keys.into_iter().collect();
        } else {
            self.selected.clear();
            self.selected.insert(self.assets()[idx].device_path());
            self.anchor = Some(pos);
        }
        self.reveal(pos, false);
    }

    /// Copies the focused item (edited version if any) to a temp folder and
    /// opens it with the system viewer or player.
    pub(crate) fn open_focused(&mut self) {
        let (Some(udid), Some(idx)) = (self.udid().map(str::to_owned), self.focused) else {
            return;
        };
        let a = &self.assets()[idx];
        let plan = a.save_plan();
        let ext = plan.from_ext.to_ascii_lowercase();
        let name = if plan.uses_edited {
            format!("{}-edited.{ext}", a.stem())
        } else {
            a.filename.clone()
        };
        let dir = self.paths.open_for(&udid, a);
        self.status = Some(format!("Opening {name}…"));
        let _ = self.commands.try_send(Command::Open {
            udid,
            device_path: plan.source,
            name,
            dir,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{TILE_DEFAULT, TILE_MAX, TILE_MIN, layout, zoomed};

    #[test]
    fn tiles_fill_the_row() {
        // 8 px gaps: 4 tiles of 132 need 552 px.
        assert_eq!(layout(600., 132.), (4, 144.));
        // Bigger zoom, fewer columns.
        assert_eq!(layout(600., 280.), (2, 296.));
        // Narrower than one tile: one tile, never zero.
        assert_eq!(layout(50., 132.).0, 1);
    }

    #[test]
    fn zoom_steps_stay_in_range() {
        assert_eq!(zoomed(TILE_DEFAULT, 1), 192.);
        assert_eq!(zoomed(TILE_DEFAULT, -1), 133.);
        assert_eq!(zoomed(TILE_DEFAULT, 20), TILE_MAX);
        assert_eq!(zoomed(TILE_DEFAULT, -20), TILE_MIN);
    }
}
