//! Toolbar pieces: the Save-to field and the copy buttons.

use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _};
use gpui_kit::*;

use crate::view::{MainView, btn};

use super::*;

impl MainView {
    // ---- toolbar pieces ----------------------------------------------

    /// The Save-to folder: its path in a field, with a button to change
    /// it. `None` until a folder is known.
    pub(crate) fn render_save_to(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dest = self.dest_root()?;
        let theme = cx.theme().clone();
        Some(
            // The path gives way first when the toolbar is short of room.
            h_flex()
                .id("save-to")
                .min_w_0()
                .max_w(px(420.))
                .h(px(28.))
                .pl_3()
                .pr_1()
                .gap_2()
                .items_center()
                .rounded(px(crate::palette::RADIUS))
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .text_sm()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child("Save to"),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(tilde(&dest)),
                )
                .child(
                    div().flex_shrink_0().child(
                        btn("pick-dest")
                            .xsmall()
                            .ghost()
                            .icon(IconName::FolderOpen)
                            .tooltip("Choose the destination folder")
                            .on_click(cx.listener(|this, _, _, cx| this.pick_destination(cx))),
                    ),
                )
                .into_any_element(),
        )
    }

    /// Copy new and Copy selected, once a Save-to folder is known.
    pub(crate) fn render_copy_buttons(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.dest_root()?;
        let n = self.copyable_selected();
        let new = self.new_count;
        Some(
            h_flex()
                .flex_shrink_0()
                .gap_2()
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
                .child(
                    btn("copy-new")
                        .small()
                        .outline()
                        .icon(IconName::CopyPlus)
                        .label(format!("Copy new ({new})"))
                        .tooltip("Copy everything not copied yet (not Hidden or Recently Deleted)")
                        .disabled(new == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.copy_new();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}
