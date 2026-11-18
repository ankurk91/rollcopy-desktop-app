//! The toolbar: menu, Copy new / Copy selected, the Save-to folder,
//! search, Reload, Preferences, theme and GitHub. The phone itself is in
//! the sidebar.

use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, TitleBar};
use gpui_kit::*;
use rollcopy_device::Phase;

use crate::prefs::Prefs;
use crate::theme::ThemeChoice;

use crate::view::*;
use crate::widgets::*;

impl MainView {
    /// The toolbar. Where Rollcopy draws its own window frame (GNOME on
    /// Wayland), it becomes the title bar: drag it to move the window,
    /// double-click to maximise, with minimise/maximise/close at the end.
    pub(crate) fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let bar = self.render_toolbar_row(cx);
        if client_decorated(window) {
            TitleBar::new()
                .on_close_window(|_, window, cx| window.dispatch_action(Box::new(CloseWindow), cx))
                // Fill the bar but leave room for the window buttons.
                .child(bar.flex_1().min_w_0().pr_2())
                .into_any_element()
        } else {
            let theme = cx.theme().clone();
            bar.w_full()
                .px_3()
                .py(px(6.))
                .bg(theme.title_bar)
                .border_b_1()
                .border_color(theme.border)
                .into_any_element()
        }
    }

    /// Menu and copy buttons on the left, the Save-to folder in the
    /// middle, search and the app buttons on the right.
    fn render_toolbar_row(&self, cx: &mut Context<Self>) -> Div {
        let ready = matches!(self.library, LibState::Loaded { .. })
            && matches!(self.phase, Phase::Ready { .. });
        let reload = ready.then(|| {
            btn("reload")
                .xsmall()
                .ghost()
                .icon(IconName::RefreshCw)
                .tooltip("Reload from iPhone (F5)")
                .disabled(self.reloading)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reload_library();
                    cx.notify();
                }))
        });
        // Buttons never shrink; when the window is narrow the Save-to path
        // and the search box give way instead.
        let app_buttons = h_flex()
            .flex_shrink_0()
            .gap_1()
            .items_center()
            .children(reload)
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        // Saved; the Prefs observer applies it.
                        let next = ThemeChoice::from(this.config.app.theme).next();
                        Prefs::update_config(cx, |c| c.app.theme = next.into());
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
            );
        let search = ready.then(|| {
            div().w(px(220.)).min_w(px(120.)).flex_shrink_1().child(
                gpui_kit::component::input::Input::new(&self.search)
                    .small()
                    .cleanable(true)
                    .prefix(Icon::new(IconName::Search).small()),
            )
        });
        h_flex()
            .gap_2()
            .items_center()
            .min_w_0()
            .child(div().flex_shrink_0().child(self.render_app_menu()))
            .children(self.render_copy_buttons(cx))
            .child(div().flex_1().min_w(px(8.)))
            .children(self.render_save_to(cx))
            .child(div().flex_1().min_w(px(8.)))
            .children(search)
            .child(app_buttons)
    }
}
