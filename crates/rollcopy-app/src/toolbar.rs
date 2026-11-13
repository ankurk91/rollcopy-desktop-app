//! The toolbar: Save to, Copy selected, menu, Preferences, theme, GitHub,
//! and the phone with its Reload button.

use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_device::Phase;

use crate::prefs::Prefs;
use crate::theme::ThemeChoice;

use crate::view::*;
use crate::widgets::*;

impl MainView {
    pub(crate) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let save_to = self.render_save_to(cx);
        let status = match &self.phase {
            Phase::Ready { info } => info.name.clone(),
            Phase::NoDevice { .. } | Phase::UsbmuxdMissing => "No iPhone connected".into(),
            _ => "Connecting…".into(),
        };
        let ready = matches!(self.library, LibState::Loaded { .. });
        h_flex()
            .w_full()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(Icon::new(IconName::Smartphone).small())
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Rollcopy"))
            .child(div().w(px(24.)))
            .children(save_to)
            .child(div().flex_1())
            .child(self.render_app_menu())
            .child(
                btn("prefs")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip("Preferences (Ctrl+,)")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(OpenPreferences), cx)
                    }),
            )
            .child(
                btn("theme-toggle")
                    .xsmall()
                    .ghost()
                    .icon(match ThemeChoice::from(self.config.app.theme) {
                        ThemeChoice::System => IconName::SunMoon,
                        ThemeChoice::Light => IconName::Sun,
                        ThemeChoice::Dark => IconName::Moon,
                    })
                    .tooltip(ThemeChoice::from(self.config.app.theme).label())
                    .on_click(cx.listener(|this, _, window, cx| {
                        // Saved; the Prefs observer applies it.
                        let next = ThemeChoice::from(this.config.app.theme).next();
                        Prefs::update_config(cx, |c| c.app.theme = next.into());
                        let _ = window;
                        cx.notify();
                    })),
            )
            .child(
                btn("github")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Github)
                    .tooltip("Rollcopy on GitHub")
                    .on_click(|_, _, cx| cx.open_url(crate::update::REPO_URL)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
            .when(ready, |d| {
                d.child(
                    btn("reload")
                        .xsmall()
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .tooltip("Reload from iPhone (F5)")
                        .disabled(self.reloading)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reload_library();
                            cx.notify();
                        })),
                )
            })
    }
}
