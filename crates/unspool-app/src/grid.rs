//! Centre pane: header with filter chips, and the virtualised thumbnail grid
//! (PLAN §6). Only rows on screen are built, and only their thumbnails are
//! requested, so a 5,000-item library scrolls without loading everything.

use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use unspool_library::{MediaKind, Source};

use crate::tree::{Selection, month_name};
use crate::view::{Filter, LEFT_W, LibState, MainView, Thumb, muted};
use crate::worker::{Command, ThumbReq, phone_thumb_path};

pub(crate) const TILE: f32 = 132.;
const GAP: f32 = 8.;
const PAD: f32 = 16.;
/// Room for the scrollbar so the last column never sits under it.
const SCROLLBAR: f32 = 14.;

impl MainView {
    /// Tiles per row for the current window width.
    fn columns(&self, window: &Window) -> usize {
        let width = window.viewport_size().width / px(1.);
        let avail = width - LEFT_W - self.preview_w - 2. * PAD - SCROLLBAR;
        (((avail + GAP) / (TILE + GAP)).floor() as usize).max(1)
    }

    pub(crate) fn render_grid(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let cols = self.columns(window);
        self.cols = cols;
        let rows = self.visible.len().div_ceil(cols);

        let header = self.render_grid_header(cx).into_any_element();
        let body = if self.visible.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Nothing here with this filter.")
                .into_any_element()
        } else {
            uniform_list(
                "grid",
                rows,
                cx.processor(move |this, range: Range<usize>, _window, cx| {
                    this.request_thumbs(range.clone(), cols);
                    range
                        .map(|row| this.render_row(row, cols, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.grid_scroll)
            .flex_1()
            .px(px(PAD))
            .pb(px(PAD))
            .into_any_element()
        };

        v_flex()
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn render_grid_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let counts = self.filter_counts;
        let count = |f: Filter| counts[f.count_index()];

        let title = match self.selection {
            Selection::All => "All media".to_owned(),
            Selection::Year(y) => y.to_string(),
            Selection::Month(y, m) => format!("{} {y}", month_name(m)),
            Selection::Undated => "Undated".to_owned(),
            Selection::RecentlyDeleted => "Recently Deleted".to_owned(),
        };
        let mut summary = format!("{} items", self.visible.len());
        if !self.selected.is_empty() {
            summary.push_str(&format!(" · {} selected", self.selected.len()));
        }
        if let LibState::Loaded { library, .. } = &self.library
            && let Source::DcimScan { reason } = &library.source
        {
            summary.push_str(&format!(" · from DCIM folder ({reason})"));
        }

        let chip = |id: &'static str, label: &str, f: Filter, n: usize, cx: &mut Context<Self>| {
            let active = self.filter == f;
            Button::new(id)
                .small()
                .label(format!("{label} {n}"))
                .when(active, |b| b.primary())
                .when(!active, |b| b.ghost())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.filter = f;
                    this.recompute_visible();
                    // Keep arrow keys on the grid, not on the chip.
                    window.focus(&this.focus, cx);
                    cx.notify();
                }))
        };

        v_flex()
            .px(px(PAD))
            .pt(px(PAD))
            .pb_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_baseline()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(muted(summary, &theme)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(chip("f-all", "All", Filter::All, count(Filter::All), cx))
                    .child(chip(
                        "f-photos",
                        "Photos",
                        Filter::Photos,
                        count(Filter::Photos),
                        cx,
                    ))
                    .child(chip(
                        "f-videos",
                        "Videos",
                        Filter::Videos,
                        count(Filter::Videos),
                        cx,
                    ))
                    .child(chip(
                        "f-edited",
                        "Edited",
                        Filter::Edited,
                        count(Filter::Edited),
                        cx,
                    )),
            )
            .when(self.selection == Selection::RecentlyDeleted, |d| {
                d.child(muted(
                    "Items in Recently Deleted are shown for reference and are never copied."
                        .to_owned(),
                    &theme,
                ))
            })
    }

    /// Queues thumbnails for the given rows that aren't known yet. Files
    /// already in the local cache are used directly without asking the phone.
    fn request_thumbs(&mut self, rows: Range<usize>, cols: usize) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        let start = rows.start * cols;
        let end = (rows.end * cols).min(self.visible.len());
        let mut wanted = Vec::new();
        for pos in start..end {
            let a = &self.assets()[self.visible[pos]];
            let key = a.device_path();
            if self.thumbs.contains_key(&key) {
                continue;
            }
            let dest = self.paths.thumb(&udid, a);
            if dest.exists() {
                self.thumbs.insert(key, Thumb::Ready(dest));
            } else {
                wanted.push(ThumbReq {
                    key: key.clone(),
                    source: phone_thumb_path(a),
                    dest,
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

    fn render_row(&self, row: usize, cols: usize, cx: &mut Context<Self>) -> AnyElement {
        let start = row * cols;
        let end = (start + cols).min(self.visible.len());
        h_flex()
            .h(px(TILE + GAP))
            .gap(px(GAP))
            .items_start()
            .children((start..end).map(|pos| self.render_tile(pos, cx)))
            .into_any_element()
    }

    fn render_tile(&self, pos: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let idx = self.visible[pos];
        let a = &self.assets()[idx];
        let key = a.device_path();
        let selected = self.selected.contains(&key);
        let focused = self.focused == Some(idx);

        let image = match self.thumbs.get(&key) {
            Some(Thumb::Ready(path)) => img(path.clone())
                .size_full()
                .object_fit(ObjectFit::Cover)
                .into_any_element(),
            Some(Thumb::Missing) => div()
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

        let badge = |text: String| {
            div()
                .px_1()
                .rounded_sm()
                .bg(hsla(0., 0., 0., 0.6))
                .text_color(white())
                .text_xs()
                .child(text)
        };

        div()
            .id(("tile", pos))
            .size(px(TILE))
            .flex_shrink_0()
            .relative()
            .rounded_md()
            .overflow_hidden()
            .bg(theme.muted)
            .cursor_pointer()
            .border_2()
            .border_color(if selected {
                theme.primary
            } else if focused {
                theme.ring
            } else {
                transparent_black()
            })
            .when(a.trashed, |d| d.opacity(0.4))
            .child(image)
            .when(a.kind == MediaKind::Video, |d| {
                let label = match a.duration_secs {
                    Some(s) => format!("▶ {}:{:02}", s as u64 / 60, s as u64 % 60),
                    None => "▶".to_owned(),
                };
                d.child(div().absolute().bottom_1().left_1().child(badge(label)))
            })
            .child(
                h_flex()
                    .absolute()
                    .top_1()
                    .right_1()
                    .gap_1()
                    .when(a.live_photo, |d| d.child(badge("LIVE".into())))
                    .when(a.is_edited(), |d| d.child(badge("✎".into()))),
            )
            .when(selected, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_1()
                        .left_1()
                        .size(px(20.))
                        .rounded_full()
                        .bg(theme.primary)
                        .text_color(theme.primary_foreground)
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("✓"),
                )
            })
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.click_tile(pos, ev.modifiers(), ev.click_count());
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .into_any_element()
    }

    /// Click, Ctrl+click, Shift+click selection (PLAN §6). A double-click
    /// opens the item in the system viewer.
    fn click_tile(&mut self, pos: usize, mods: Modifiers, clicks: usize) {
        let Some(&idx) = self.visible.get(pos) else {
            return;
        };
        self.focused = Some(idx);
        self.status = None;
        if clicks >= 2 {
            self.open_focused();
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
        self.grid_scroll
            .scroll_to_item(pos / self.cols.max(1), ScrollStrategy::Nearest);
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
        self.status = Some(format!("Opening {name}…"));
        let _ = self.commands.try_send(Command::Open {
            udid,
            device_path: plan.source,
            name,
        });
    }
}
