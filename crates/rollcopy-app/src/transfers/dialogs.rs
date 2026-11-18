//! The "already exists" and "Copy all?" prompts.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::Asset;
use rollcopy_queue::QueueCommand;

use crate::view::{MainView, btn, muted};

use super::*;

impl MainView {
    /// "Copy all N items?" before copying everything shown.
    pub(crate) fn render_copy_all_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.copy_all_prompt {
            return None;
        }
        let theme = cx.theme().clone();
        let items: Vec<&Asset> = self
            .visible
            .iter()
            .map(|&i| &self.assets()[i])
            .filter(|a| !a.trashed)
            .collect();
        let n = items.len();
        let bytes: u64 = items.iter().filter_map(|a| a.original_size).sum();
        let already = items
            .iter()
            .filter(|a| self.is_copied(&a.device_path()))
            .count();
        let dest = self.dest_root().map(|d| tilde(&d)).unwrap_or_default();
        let mut body = format!(
            "{n} item{} ({}) will be copied to {dest}.",
            if n == 1 { "" } else { "s" },
            human_size(bytes)
        );
        if already > 0 {
            body.push_str(&format!(
                " {already} of them are copied already; {}",
                match self.config.destination.on_conflict {
                    crate::config::ConflictPref::Ask => "you'll be asked what to do with those.",
                    crate::config::ConflictPref::Skip => "those are skipped.",
                    crate::config::ConflictPref::Overwrite => "those are copied again.",
                }
            ));
        }
        let card =
            v_flex()
                .w(px(460.))
                .p_5()
                .gap_3()
                .rounded_lg()
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .shadow_lg()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                    "Copy all {n} item{}?",
                    if n == 1 { "" } else { "s" }
                )))
                .child(muted(body, &theme))
                .child(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(btn("copy-all-cancel").outline().label("Cancel").on_click(
                            cx.listener(|this, _, window, cx| {
                                this.copy_all_prompt = false;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }),
                        ))
                        .child(
                            btn("copy-all-yes")
                                .primary()
                                .icon(IconName::Copy)
                                .label("Copy all")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.copy_all_prompt = false;
                                    this.copy_visible();
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                })),
                        ),
                );
        Some(crate::about::overlay(
            "copy-all-backdrop",
            card.into_any_element(),
        ))
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
}
