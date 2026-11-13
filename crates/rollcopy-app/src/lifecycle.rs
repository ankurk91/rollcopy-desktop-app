//! Closing and quitting: the "copies are still running" prompt and saving
//! the window state.

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::*;

use crate::prefs::Prefs;

use crate::view::*;
use crate::widgets::*;

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
        let list_columns = self.list_widths.clone();
        let transfer_columns = self.transfers.widths.clone();
        let transfers_height = self.transfers.height;
        Prefs::update_ui(cx, |ui| {
            ui.list_columns = list_columns;
            ui.transfer_columns = transfer_columns;
            ui.transfers_height = transfers_height;
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
    pub(crate) fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
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

    pub(crate) fn render_quit_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
