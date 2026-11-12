//! ☰ app menu actions, About dialog (PLAN §8b) and Check for updates (§8c).

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::prefs::Prefs;
use crate::update::{self, UpdateCheck};
use crate::view::{
    CheckForUpdates, MainView, OpenPreferences, Quit, ReportIssue, ShowAbout, ShowShortcuts,
    ViewOnGitHub, btn, muted,
};

/// Path the app icon is served under (see `main.rs` asset source).
pub const APP_ICON: &str = "rollcopy/app-icon.svg";
const GIT_HASH: &str = env!("ROLLCOPY_GIT_HASH");
/// Once-a-day limit for the automatic startup check.
const DAY_SECS: i64 = 24 * 60 * 60;

const CREDITS: &[(&str, &str)] = &[
    ("gpui, gpui-kit, gpui-component", "Apache-2.0"),
    ("idevice", "MIT"),
    ("libheif", "LGPL-3.0"),
    ("libde265", "LGPL-3.0"),
    ("FFmpeg (run as a separate program)", "LGPL/GPL"),
    ("Lucide icons", "ISC/MIT"),
];

/// Keyboard shortcuts (shown with F1 or ☰ → Keyboard Shortcuts; keep the
/// README table in sync).
pub(crate) const SHORTCUTS: &[(&str, &str)] = &[
    ("← → ↑ ↓", "Move between items"),
    ("Shift + arrows", "Extend the selection"),
    (
        "Click / Ctrl+click / Shift+click",
        "Select / add to / extend the selection",
    ),
    ("Ctrl+A", "Select all items shown"),
    ("Esc", "Clear the selection, or close the viewer"),
    (
        "Space / Enter / double-click",
        "View the item full-window (videos: open the player)",
    ),
    ("Ctrl+,", "Preferences"),
    ("F1", "This list"),
    ("Ctrl+Q", "Quit"),
];

/// State of the update dialog.
pub(crate) enum UpdateUi {
    Checking,
    Done(UpdateCheck),
}

pub(crate) fn version_line() -> String {
    format!("Version {} ({GIT_HASH})", update::current_version())
}

impl MainView {
    /// The ☰ button and its menu (PLAN §6 toolbar).
    pub(crate) fn render_app_menu(&self) -> impl IntoElement {
        btn("app-menu")
            .xsmall()
            .ghost()
            .icon(IconName::Menu)
            .tooltip("Menu")
            .dropdown_menu(|menu, _, _| {
                menu.menu("Preferences…", Box::new(OpenPreferences))
                    .menu("Check for Updates…", Box::new(CheckForUpdates))
                    .separator()
                    .menu("View on GitHub", Box::new(ViewOnGitHub))
                    .menu("Report an Issue", Box::new(ReportIssue))
                    .separator()
                    .menu("Keyboard Shortcuts", Box::new(ShowShortcuts))
                    .menu("About Rollcopy", Box::new(ShowAbout))
                    .menu("Quit", Box::new(Quit))
            })
    }

    /// Starts an update check. `manual` shows the dialog with every outcome;
    /// the automatic startup check stays silent unless there's an update.
    pub(crate) fn check_for_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        if manual {
            self.update_ui = Some(UpdateUi::Checking);
            self.about_open = false;
        }
        let task = cx.background_executor().spawn(async { update::check() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |view, cx| {
                let now = chrono::Utc::now().timestamp();
                Prefs::update_ui(cx, |ui| ui.last_update_check = Some(now));
                if manual {
                    view.update_ui = Some(UpdateUi::Done(result));
                } else if let UpdateCheck::Newer { version, url, .. } = result {
                    view.update_banner = Some((version, url));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Automatic check on startup: only if enabled and not done in a day.
    pub(crate) fn maybe_check_on_startup(&mut self, cx: &mut Context<Self>) {
        let prefs = Prefs::get(cx);
        if !prefs.config.app.check_updates_on_startup {
            return;
        }
        let now = chrono::Utc::now().timestamp();
        if prefs
            .ui
            .last_update_check
            .is_some_and(|t| now - t < DAY_SECS)
        {
            return;
        }
        self.check_for_updates(false, cx);
    }

    fn debug_info(&self) -> String {
        let os = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines()
                    .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                    .map(|v| v.trim_matches('"').to_owned())
            })
            .unwrap_or_else(|| "unknown".into());
        let desktop = format!(
            "{} ({})",
            std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into()),
            std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
        );
        let usbmuxd = std::process::Command::new("usbmuxd")
            .arg("--version")
            .output()
            .ok()
            .map(|o| {
                let text = String::from_utf8_lossy(&o.stdout).to_string()
                    + &String::from_utf8_lossy(&o.stderr);
                text.lines().next().unwrap_or("").trim().to_owned()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "not found".into());
        let phone = match &self.phase {
            rollcopy_device::Phase::Ready { info } => {
                format!(
                    "{} on iOS {} ({})",
                    info.model, info.ios_version, info.build_version
                )
            }
            _ => "not connected".into(),
        };
        format!(
            "Rollcopy {} ({GIT_HASH})\nOS: {os}\nDesktop: {desktop}\nusbmuxd: {usbmuxd}\nPhone: {phone}",
            update::current_version()
        )
    }

    pub(crate) fn render_about(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.about_open {
            return None;
        }
        let theme = cx.theme().clone();
        let credits_open = self.credits_open;
        let card = v_flex()
            .w(px(440.))
            .p_6()
            .gap_2()
            .items_center()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .child(img(APP_ICON).size(px(96.)))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Rollcopy"),
            )
            .child(muted(version_line(), &theme))
            .child(
                div()
                    .text_center()
                    .child("Copy photos and videos from your iPhone to Linux over a USB cable."),
            )
            .child(
                h_flex()
                    .mt_2()
                    .gap_2()
                    .child(
                        btn("about-github")
                            .small()
                            .outline()
                            .label("GitHub")
                            .on_click(|_, _, cx| cx.open_url(update::REPO_URL)),
                    )
                    .child(
                        btn("about-issue")
                            .small()
                            .outline()
                            .label("Report an issue")
                            .on_click(|_, _, cx| cx.open_url(update::ISSUES_URL)),
                    )
                    .child(
                        btn("about-updates")
                            .small()
                            .outline()
                            .label("Check for updates")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.check_for_updates(true, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .mt_2()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("Licence: MIT ·")
                    .child(
                        div()
                            .id("credits-toggle")
                            .cursor_pointer()
                            .text_color(theme.link)
                            .child(if credits_open {
                                "Credits ▾"
                            } else {
                                "Credits ▸"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.credits_open = !this.credits_open;
                                cx.notify();
                            })),
                    ),
            )
            .when(credits_open, |d| {
                d.child(
                    v_flex()
                        .w_full()
                        .mt_1()
                        .gap_0p5()
                        .text_xs()
                        .children(CREDITS.iter().map(|(name, lic)| {
                            h_flex()
                                .justify_between()
                                .child(*name)
                                .child(div().text_color(theme.muted_foreground).child(*lic))
                        }))
                        .child(
                            h_flex().mt_2().justify_center().child(
                                btn("copy-debug")
                                    .xsmall()
                                    .outline()
                                    .icon(IconName::Copy)
                                    .label("Copy debug info")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            this.debug_info(),
                                        ));
                                        this.about_note = Some("Debug info copied".into());
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
            })
            .when_some(self.about_note.clone(), |d, n| d.child(muted(n, &theme)))
            .child(
                h_flex().mt_3().child(
                    btn("about-close")
                        .small()
                        .primary()
                        .label("Close")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.about_open = false;
                            cx.notify();
                        })),
                ),
            );
        Some(overlay("about-backdrop", card.into_any_element()))
    }

    pub(crate) fn render_shortcuts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shortcuts_open {
            return None;
        }
        let theme = cx.theme().clone();
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
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Keyboard shortcuts"),
            )
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .children(SHORTCUTS.iter().map(|(keys, what)| {
                        h_flex()
                            .gap_3()
                            .child(
                                div()
                                    .w(px(220.))
                                    .flex_shrink_0()
                                    .font_family("monospace")
                                    .child(*keys),
                            )
                            .child(div().text_color(theme.muted_foreground).child(*what))
                    })),
            )
            .child(
                h_flex().justify_end().child(
                    btn("shortcuts-close")
                        .small()
                        .primary()
                        .label("Close")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.shortcuts_open = false;
                            cx.notify();
                        })),
                ),
            );
        Some(overlay("shortcuts-backdrop", card.into_any_element()))
    }

    pub(crate) fn render_update_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.update_ui.as_ref()?;
        let theme = cx.theme().clone();
        let current = update::current_version();
        let mut card = v_flex()
            .w(px(480.))
            .p_5()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg();
        let mut download: Option<String> = None;
        match state {
            UpdateUi::Checking => {
                card = card.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Checking for updates…"),
                );
            }
            UpdateUi::Done(UpdateCheck::Newer {
                version,
                notes,
                url,
            }) => {
                download = Some(url.clone());
                card = card
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                        "Rollcopy {version} is available (you have {current})"
                    )))
                    .when(!notes.trim().is_empty(), |d| {
                        d.child(
                            div()
                                .id("release-notes")
                                .max_h(px(240.))
                                .overflow_y_scroll()
                                .p_2()
                                .rounded_md()
                                .bg(theme.muted)
                                .text_sm()
                                .child(notes.clone()),
                        )
                    });
            }
            UpdateUi::Done(UpdateCheck::UpToDate { current }) => {
                card = card.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("You're on the latest version ({current})")),
                );
            }
            UpdateUi::Done(UpdateCheck::NoReleases) => {
                card = card
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("No releases published yet"),
                    )
                    .child(muted(
                        format!("You're running {current}. Releases will appear on GitHub."),
                        &theme,
                    ));
            }
            UpdateUi::Done(UpdateCheck::Failed { reason }) => {
                card = card
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Couldn't check for updates — try again later"),
                    )
                    .child(muted(format!("Details: {reason}"), &theme));
            }
        }
        card = card.child(
            h_flex()
                .gap_2()
                .justify_end()
                .when_some(download, |d, url| {
                    d.child(
                        btn("update-download")
                            .primary()
                            .label("Download")
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                })
                .child(
                    btn("update-close")
                        .outline()
                        .label("Close")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_ui = None;
                            cx.notify();
                        })),
                ),
        );
        Some(overlay("update-backdrop", card.into_any_element()))
    }

    /// Small banner after a startup check found an update.
    pub(crate) fn render_update_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (version, url) = self.update_banner.clone()?;
        let theme = cx.theme().clone();
        Some(
            h_flex()
                .w_full()
                .px_4()
                .py_1()
                .gap_2()
                .items_center()
                .text_sm()
                .bg(theme.primary.opacity(0.15))
                .child(format!("Rollcopy {version} is available."))
                .child(
                    btn("banner-download")
                        .xsmall()
                        .outline()
                        .label("Download")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
                .child(div().flex_1())
                .child(
                    btn("banner-close")
                        .xsmall()
                        .ghost()
                        .icon(IconName::X)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.update_banner = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}

fn overlay(id: &'static str, card: AnyElement) -> AnyElement {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .bg(hsla(0., 0., 0., 0.45))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(card)
        .into_any_element()
}
