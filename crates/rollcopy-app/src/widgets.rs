//! Small UI helpers shared by the panes.

use gpui_kit::base::v_flex;
use gpui_kit::component::Theme;
use gpui_kit::component::button::Button;
use gpui_kit::*;

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

pub(crate) fn detail(label: &str, value: &str, theme: &Theme) -> impl IntoElement {
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

/// A button with the hand cursor (gpui-component defaults to the arrow).
pub(crate) fn btn(id: impl Into<ElementId>) -> Button {
    Button::new(id).cursor_pointer()
}
