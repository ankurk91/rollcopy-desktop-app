//! List view of the library: one row per item, no thumbnails (faster, and
//! nothing to see over your shoulder). Columns: `columns::LIBRARY`.

use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::MediaKind;

use crate::columns::{LIBRARY, ResizeColumn, TRANSFERS, TableKind};
use crate::view::MainView;

pub(crate) const ROW_H: f32 = 32.;

impl MainView {
    pub(crate) fn render_list_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let shown = LIBRARY.fit(&self.list_widths, self.list_row_w);
        div()
            .mx(px(crate::grid::PAD))
            .border_b_1()
            .border_color(theme.border)
            .child(
                LIBRARY
                    .row(
                        &shown,
                        div().into_any_element(),
                        LIBRARY.header_cells(),
                        None,
                    )
                    .id("list-header")
                    .h(px(ROW_H))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .on_drag_move(
                        cx.listener(|this, ev: &DragMoveEvent<ResizeColumn>, _, cx| {
                            this.resize_column(TableKind::Library, ev, cx)
                        }),
                    ),
            )
    }

    /// A column edge was dragged in the header of `table`.
    pub(crate) fn resize_column(
        &mut self,
        table: TableKind,
        ev: &DragMoveEvent<ResizeColumn>,
        cx: &mut Context<Self>,
    ) {
        let drag = ev.drag(cx).clone();
        if drag.table != table {
            return;
        }
        let x = (ev.event.position.x - ev.bounds.origin.x) / px(1.);
        let row_w = ev.bounds.size.width / px(1.);
        let (spec, widths) = match table {
            TableKind::Library => (&LIBRARY, &mut self.list_widths),
            TableKind::Transfers => (&TRANSFERS, &mut self.transfers.widths),
        };
        // Start from what's on screen, so a squeezed column doesn't jump.
        let mut shown = spec.fit(widths, row_w);
        shown[drag.col] = spec.dragged(&shown, drag.col, x, row_w);
        *widths = shown;
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

        let shown = LIBRARY.fit(&self.list_widths, self.list_row_w);
        LIBRARY
            .row(&shown, mark, cells, None)
            .id(("row", pos))
            .h(px(ROW_H))
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(focused, |d| d.border_color(theme.ring))
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
