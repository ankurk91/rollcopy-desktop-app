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

/// Whether this window draws its own title bar and buttons. GNOME on
/// Wayland doesn't draw them for apps (no server-side decorations), so
/// Rollcopy must; window managers that do (Cinnamon, KDE, X11) keep theirs.
pub(crate) fn client_decorated(window: &Window) -> bool {
    matches!(window.window_decorations(), Decorations::Client { .. })
}

/// Which decorations to ask for. Server-side by default; debug builds can
/// try the client-side look on any desktop with `ROLLCOPY_CSD=1`.
pub(crate) fn requested_decorations() -> Option<WindowDecorations> {
    if cfg!(debug_assertions) && std::env::var_os("ROLLCOPY_CSD").is_some() {
        Some(WindowDecorations::Client)
    } else {
        None
    }
}

/// The window frame when Rollcopy draws its own (GNOME on Wayland): a 1 px
/// edge with resize handles, and no drawn shadow. A shadow needs a
/// see-through margin around the window; with software rendering (virtual
/// machines, no GPU) that margin came out as a solid grey band, so there is
/// none. Elsewhere this is a plain wrapper.
pub(crate) fn window_frame() -> gpui_kit::component::WindowBorder {
    gpui_kit::component::window_border()
        .shadow_size(px(0.))
        .resize_hit_size(px(6.))
}
