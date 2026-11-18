//! The transfer dock at the bottom: one status line, and the list of
//! files when expanded.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_queue::{JobState, QueueCommand};

use crate::columns::{ResizeColumn, TRANSFERS, TableKind};
use crate::view::{MainView, btn, muted};

use super::*;

impl MainView {
    // ---- bottom panel ----------------------------------------------

    /// The panel's top edge was dragged.
    pub(crate) fn resize_transfers(
        &mut self,
        ev: &DragMoveEvent<ResizeTransfers>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let win_h = window.viewport_size().height / px(1.);
        let y = ev.event.position.y / px(1.);
        // Keep at least half the window for the library.
        let max = (win_h * 0.6).max(PANEL_MIN_H);
        self.transfers.height = (win_h - y - PANEL_CHROME_H).clamp(PANEL_MIN_H, max);
        cx.notify();
    }

    pub(crate) fn render_transfers(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.transfers.rows.is_empty() {
            return None;
        }
        // Rows sit between the panel's 16 px side paddings.
        self.transfers.row_w = window.viewport_size().width / px(1.) - 32.;
        let theme = cx.theme().clone();
        let t = &self.transfers;
        let (done, failed, active, total) = t.counts();

        let progress = (active > 0)
            .then(|| overall(&t.rows, t.batch_start))
            .flatten();
        let paused = t.paused.clone();
        let current = t
            .rows
            .iter()
            .find(|r| {
                matches!(
                    r.state,
                    JobState::Reading | JobState::Converting | JobState::Verifying
                )
            })
            .map(|r| format!("{} {}", state_label(&r.state, r.progress), r.name));
        let status_icon = if active > 0 && paused.is_some() {
            Icon::new(IconName::Pause)
                .small()
                .text_color(theme.warning)
                .into_any_element()
        } else if active > 0 {
            Spinner::new().small().into_any_element()
        } else if failed > 0 {
            Icon::new(IconName::CircleAlert)
                .small()
                .text_color(theme.danger)
                .into_any_element()
        } else {
            Icon::new(IconName::CheckCheck)
                .small()
                .text_color(theme.success)
                .into_any_element()
        };
        let bar = |f: f32| {
            div()
                .flex_shrink_0()
                .w(px(140.))
                .h(px(4.))
                .rounded_full()
                .bg(theme.border)
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .w(relative(f))
                        .bg(if paused.is_some() {
                            theme.warning
                        } else {
                            theme.primary
                        }),
                )
        };
        let one_line = |text: String, color: Hsla| {
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_sm()
                .text_color(color)
                .child(text)
        };

        // The dock's one line: status, progress, the current file, then the
        // controls. Clicking it shows or hides the list of files.
        let header = h_flex()
            .id("transfers-header")
            .w_full()
            .h(px(34.))
            .px_3()
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
                .xsmall()
                .text_color(theme.muted_foreground),
            )
            .child(div().flex_shrink_0().child(status_icon))
            .child(
                div()
                    .flex_shrink_0()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(headline(done, failed, active, total, paused.is_some())),
            )
            .when_some(progress.as_ref(), |d, p| {
                d.child(bar(p.fraction())).child(
                    div()
                        .flex_shrink_0()
                        .child(one_line(p.label(), theme.muted_foreground)),
                )
            })
            .when_some(paused.clone(), |d, reason| {
                d.child(one_line(reason, theme.warning))
            })
            .when_some(current, |d, c| d.child(one_line(c, theme.muted_foreground)))
            .child(div().flex_1())
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_1()
                    .items_center()
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
                            .icon(IconName::FileDown)
                            .tooltip("Export error report")
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
                                .tooltip("Open the Save-to folder")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    spawn_xdg_open(dest.clone());
                                    cx.stop_propagation();
                                })),
                        )
                    }),
            );

        let mut panel = v_flex()
            .relative()
            .w_full()
            .flex_shrink_0()
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.status_bar)
            .child(header);
        if let Some(note) = t.note.clone() {
            panel = panel.child(div().px_3().pb_1().child(muted(note, &theme)));
        }
        if t.expanded {
            // Drag the top edge to make the panel taller or shorter.
            panel = panel.child(
                resize_handle::<ResizeTransfers, ResizeTransfers>("transfers-edge", Axis::Vertical)
                    .inside(HandleEdge::Leading)
                    .on_drag(ResizeTransfers, |drag, _, _, cx| cx.new(|_| drag.clone())),
            );
            // Column headings: same widths as the rows below.
            let shown = TRANSFERS.fit(&t.widths, t.row_w);
            // The list sits on the content shade, below the darker bar.
            let mut list = v_flex()
                .bg(theme.background)
                .border_t_1()
                .border_color(theme.border);
            list = list.child(
                div().px_4().child(
                    TRANSFERS
                        .row(
                            &shown,
                            div().into_any_element(),
                            TRANSFERS.header_cells(),
                            None,
                        )
                        .id("transfers-header-row")
                        .h(px(22.))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .on_drag_move(cx.listener(
                            |this, ev: &DragMoveEvent<ResizeColumn>, _, cx| {
                                this.resize_column(TableKind::Transfers, ev, cx)
                            },
                        )),
                ),
            );
            list = list.child(
                div()
                    .relative()
                    .h(px(t.height))
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
            panel = panel.child(list);
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
        let failed = matches!(r.state, JobState::Failed { .. });
        let cells = vec![
            div().child(r.name.clone()).into_any_element(),
            div()
                .text_color(theme.muted_foreground)
                .child(folder)
                .into_any_element(),
            div()
                .text_color(if failed {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(state_label(&r.state, r.progress))
                .into_any_element(),
            div()
                .text_color(theme.muted_foreground)
                .child(
                    r.took
                        .or_else(|| r.started.map(|s| s.elapsed()))
                        .map(took_label)
                        .unwrap_or_default(),
                )
                .into_any_element(),
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    r.finished_at
                        .map(|t| t.format("%-d %b %H:%M:%S").to_string())
                        .unwrap_or_default(),
                )
                .into_any_element(),
        ];
        let actions = h_flex()
            .gap_1()
            .when(failed, |d| {
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
            .into_any_element();
        let shown = TRANSFERS.fit(&self.transfers.widths, self.transfers.row_w);
        TRANSFERS
            .row(
                &shown,
                div().text_color(color).child(icon).into_any_element(),
                cells,
                Some(actions),
            )
            .id(("transfer", i))
            .h(px(ROW_H))
            .rounded(px(crate::palette::RADIUS))
            .text_sm()
            .hover(|s| s.bg(theme.muted))
            .into_any_element()
    }
}
