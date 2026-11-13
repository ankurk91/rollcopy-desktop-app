//! List view of the library: one row per item, no thumbnails (faster, and
//! nothing to see over your shoulder). Header and rows are built by the
//! same function from the same column widths, so they always line up;
//! columns are resized by dragging their right edge in the header.

use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::MediaKind;

use crate::view::MainView;

pub(crate) const ROW_H: f32 = 32.;
/// Space inside a row before the first column.
const ROW_PAD: f32 = 8.;
/// Fixed first column: selection tick, photo/video icon, copied mark.
const MARK_W: f32 = 64.;

/// Resizable columns: (title, default width, minimum width). The last one
/// also takes any room left over.
pub(crate) const COLUMNS: [(&str, f32, f32); 5] = [
    ("Name", 220., 80.),
    ("Taken", 150., 80.),
    ("Size", 110., 60.),
    ("File size", 90., 60.),
    ("Tags", 140., 60.),
];

/// Column widths from saved state, or the defaults when they don't fit.
pub(crate) fn widths_from(saved: &[f32]) -> Vec<f32> {
    if saved.len() == COLUMNS.len() {
        saved
            .iter()
            .zip(COLUMNS)
            .map(|(w, (_, _, min))| w.max(min))
            .collect()
    } else {
        COLUMNS.iter().map(|(_, w, _)| *w).collect()
    }
}

/// x of the left edge of column `col`, relative to the row's left edge.
fn column_start(widths: &[f32], col: usize) -> f32 {
    ROW_PAD + MARK_W + widths[..col].iter().sum::<f32>()
}

/// New width for column `col` when its edge is dragged to `x` (relative to
/// the row's left edge).
pub(crate) fn dragged_width(widths: &[f32], col: usize, x: f32) -> f32 {
    (x - column_start(widths, col)).clamp(COLUMNS[col].2, 1200.)
}

/// Drag payload for a column edge.
#[derive(Clone)]
pub(crate) struct ResizeColumn(pub usize);

impl Render for ResizeColumn {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// One row (header or item) laid out on the column widths.
fn row_frame(widths: &[f32], mark: AnyElement, cells: Vec<AnyElement>) -> Div {
    let last = cells.len().saturating_sub(1);
    h_flex()
        .h(px(ROW_H))
        .w_full()
        .pl(px(ROW_PAD))
        .items_center()
        .child(h_flex().w(px(MARK_W)).flex_shrink_0().gap_1().child(mark))
        .children(cells.into_iter().enumerate().map(|(i, cell)| {
            div()
                .relative()
                .h_full()
                .flex()
                .items_center()
                .pr_2()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .map(|d| {
                    if i == last {
                        d.flex_1().min_w(px(widths[i]))
                    } else {
                        d.w(px(widths[i])).flex_shrink_0()
                    }
                })
                .child(cell)
        }))
}

impl MainView {
    pub(crate) fn render_list_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let cells = COLUMNS
            .iter()
            .enumerate()
            .map(|(i, (title, _, _))| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .child(*title)
                    .child(
                        resize_handle::<ResizeColumn, ResizeColumn>(
                            ("col-edge", i),
                            Axis::Horizontal,
                        )
                        .inside(HandleEdge::Trailing)
                        .on_drag(ResizeColumn(i), |drag, _, _, cx| cx.new(|_| drag.clone())),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .mx(px(crate::grid::PAD))
            .border_b_1()
            .border_color(theme.border)
            .child(
                row_frame(&self.list_widths, div().into_any_element(), cells)
                    .id("list-header")
                    .border_1()
                    .border_color(transparent_black())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .on_drag_move(cx.listener(Self::resize_list_column)),
            )
    }

    fn resize_list_column(
        &mut self,
        ev: &DragMoveEvent<ResizeColumn>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let col = ev.drag(cx).0;
        let x = (ev.event.position.x - ev.bounds.origin.x) / px(1.);
        self.list_widths[col] = dragged_width(&self.list_widths, col, x);
        cx.notify();
    }

    pub(crate) fn render_list_row(&self, pos: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme: Theme = cx.theme().clone();
        let idx = self.visible[pos];
        let a = &self.assets()[idx];
        let key = a.device_path();
        let selected = self.selected.contains(&key);
        let focused = self.focused == Some(idx);
        let muted = theme.muted_foreground;

        let taken = a
            .captured_local()
            .map(|d| d.format("%-d %b %Y %H:%M").to_string())
            .unwrap_or_else(|| "Undated".into());
        let size = match (a.width, a.height, a.kind, a.duration_secs) {
            (_, _, MediaKind::Video, Some(s)) => {
                format!("▶ {}:{:02}", s as u64 / 60, s as u64 % 60)
            }
            (Some(w), Some(h), _, _) => format!("{w} × {h}"),
            _ => String::new(),
        };
        let file = a
            .original_size
            .map(crate::transfers::human_size)
            .unwrap_or_default();
        let mut tags = Vec::new();
        if a.live_photo {
            tags.push("Live");
        }
        if a.is_edited() {
            tags.push("Edited");
        }
        if a.hidden {
            tags.push("Hidden");
        }

        let mark = h_flex()
            .gap_1p5()
            .child(
                div()
                    .w(px(12.))
                    .text_color(theme.primary)
                    .child(if selected { "✓" } else { "" }),
            )
            .child(
                Icon::new(if a.kind == MediaKind::Video {
                    IconName::Video
                } else {
                    IconName::Image
                })
                .small()
                .text_color(muted),
            )
            .when(self.is_copied(&key), |d| {
                d.child(Icon::new(IconName::RefreshCw).small().text_color(hsla(
                    96. / 360.,
                    0.62,
                    0.55,
                    1.,
                )))
            })
            .into_any_element();
        let text = |s: String, color: Option<Hsla>| {
            div()
                .when_some(color, |d, c| d.text_color(c))
                .child(s)
                .into_any_element()
        };
        let cells = vec![
            text(a.filename.clone(), None),
            text(taken, None),
            text(size, Some(muted)),
            text(file, Some(muted)),
            text(tags.join(" · "), Some(muted)),
        ];

        row_frame(&self.list_widths, mark, cells)
            .id(("row", pos))
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            // Same 1 px border as the header, so the columns line up.
            .border_1()
            .border_color(if focused {
                theme.ring
            } else {
                transparent_black()
            })
            .when(selected, |d| d.bg(theme.accent))
            .when(!selected, |d| d.hover(|s| s.bg(theme.muted)))
            .when(a.trashed, |d| d.text_color(muted))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.click_tile(pos, ev.modifiers(), ev.click_count());
                window.focus(&this.focus, cx);
                cx.notify();
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{COLUMNS, dragged_width, widths_from};

    #[test]
    fn saved_widths_are_used_when_they_fit() {
        let defaults: Vec<f32> = COLUMNS.iter().map(|c| c.1).collect();
        assert_eq!(widths_from(&[]), defaults);
        assert_eq!(widths_from(&[1., 2.]), defaults);
        let w = widths_from(&[300., 10., 120., 90., 140.]);
        assert_eq!(w, vec![300., COLUMNS[1].2, 120., 90., 140.]);
    }

    #[test]
    fn dragging_an_edge_sets_that_column() {
        let w = widths_from(&[]);
        // Name starts after the row padding and the mark column (72 px).
        assert_eq!(dragged_width(&w, 0, 72. + 250.), 250.);
        // Taken starts after Name.
        assert_eq!(dragged_width(&w, 1, 72. + 220. + 180.), 180.);
        // Never narrower than the minimum.
        assert_eq!(dragged_width(&w, 0, 0.), COLUMNS[0].2);
    }
}
