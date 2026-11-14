//! First-run help: three steps to the first copy, shown once on first
//! launch and from ☰ → Getting started.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::*;

use crate::prefs::Prefs;
use crate::view::{MainView, btn, muted};

const STEPS: [(IconName, &str, &str); 3] = [
    (
        IconName::Usb,
        "Plug in your iPhone",
        "Use a cable that carries data, straight into this computer (no hub).",
    ),
    (
        IconName::ShieldCheck,
        "Unlock it and tap Trust",
        "Enter your passcode when the iPhone asks. It's needed once per computer.",
    ),
    (
        IconName::CopyPlus,
        "Pick and copy",
        "Copy new copies everything you haven't copied yet. Or select items and press Copy selected.",
    ),
];

impl MainView {
    /// Closes the guide and remembers that it was seen.
    pub(crate) fn close_welcome(&mut self, cx: &mut Context<Self>) {
        self.welcome_open = false;
        Prefs::update_ui(cx, |ui| ui.welcomed = true);
        cx.notify();
    }

    pub(crate) fn render_welcome(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.welcome_open {
            return None;
        }
        let theme = cx.theme().clone();
        let steps = STEPS.iter().enumerate().map(|(i, (icon, title, body))| {
            h_flex()
                .gap_3()
                .items_start()
                .child(
                    div()
                        .flex_shrink_0()
                        .size(px(36.))
                        .rounded_full()
                        .bg(theme.primary)
                        .text_color(theme.primary_foreground)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(*icon).small()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("{}. {title}", i + 1)),
                        )
                        .child(muted((*body).to_owned(), &theme)),
                )
        });
        let card = v_flex()
            .w(px(480.))
            .p_6()
            .gap_4()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Welcome to Rollcopy"),
            )
            .children(steps)
            .child(muted(
                "Rollcopy only reads from your iPhone: it never changes or deletes anything there. \
                 HEIC photos become JPG and MOV videos MP4, sorted by the date they were taken."
                    .to_owned(),
                &theme,
            ))
            .child(
                h_flex().justify_end().child(
                    btn("welcome-done")
                        .primary()
                        .label("Get started")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_welcome(cx);
                            window.focus(&this.focus, cx);
                        })),
                ),
            );
        Some(crate::about::overlay(
            "welcome-backdrop",
            card.into_any_element(),
        ))
    }
}
