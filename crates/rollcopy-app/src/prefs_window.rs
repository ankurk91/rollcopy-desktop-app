//! Preferences window (PLAN §8a): a separate window with sections. Every
//! change is saved and applied at once (no OK/Apply button).

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::setting::{
    SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::{ConflictPref, LayoutPref, ThemePref, VideoPref, default_base_folder};
use crate::prefs::Prefs;
use crate::view::btn;

use crate::APP_ID;

/// The open Preferences window, so a second request focuses it instead.
#[derive(Default)]
struct PrefsWindowHandle(Option<WindowHandle<gpui_kit::base::Root>>);

impl Global for PrefsWindowHandle {}

/// Opens the Preferences window, or brings the open one to the front.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<PrefsWindowHandle>().and_then(|h| h.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(760.), px(580.)), cx)),
        app_id: Some(APP_ID.into()),
        window_min_size: Some(size(px(560.), px(400.))),
        titlebar: Some(TitlebarOptions {
            title: Some("Rollcopy Preferences".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |_, cx| cx.new(PrefsView::new)) {
        let handle = handle.downcast::<gpui_kit::base::Root>();
        cx.set_global(PrefsWindowHandle(handle));
    }
}

pub struct PrefsView {
    _prefs: Subscription,
}

impl PrefsView {
    fn new(cx: &mut Context<Self>) -> Self {
        // Re-render when a value changes (here or from the main window).
        Self {
            _prefs: cx.observe_global::<Prefs>(|_, cx| cx.notify()),
        }
    }
}

/// (value, label) pairs for a dropdown.
fn options(items: &[(&'static str, &'static str)]) -> Vec<(SharedString, SharedString)> {
    items
        .iter()
        .map(|(v, l)| (SharedString::from(*v), SharedString::from(*l)))
        .collect()
}

fn pages() -> Vec<SettingPage> {
    vec![
        SettingPage::new("Destination")
            .description("Where copies go and how they're arranged.")
            .default_open(true)
            .group(
                SettingGroup::new()
                    .item(
                        SettingItem::render(|_, _, cx| base_folder_row(cx))
                            .description("Each phone gets its own folder inside this one."),
                    )
                    .item(SettingItem::new(
                        "Subfolder per device",
                        SettingField::switch(
                            |cx| Prefs::get(cx).config.destination.subfolder_per_device,
                            |v, cx| {
                                Prefs::update_config(cx, |c| c.destination.subfolder_per_device = v)
                            },
                        ),
                    ))
                    .item(
                        SettingItem::new(
                            "Folder layout",
                            SettingField::dropdown(
                                options(&[
                                    ("by-date", "By date"),
                                    ("flat", "One folder"),
                                ]),
                                |cx| match Prefs::get(cx).config.destination.layout {
                                    LayoutPref::ByDate => "by-date".into(),
                                    LayoutPref::Flat => "flat".into(),
                                },
                                |v, cx| {
                                    let layout = if v == "flat" {
                                        LayoutPref::Flat
                                    } else {
                                        LayoutPref::ByDate
                                    };
                                    Prefs::update_config(cx, |c| c.destination.layout = layout)
                                },
                            ),
                        )
                        .description(
                            "By date: 2026/11/images/IMG_0001.JPG, using the date each item was \
                             taken in its own time zone. One folder: everything side by side.",
                        ),
                    )
                    .item(SettingItem::new(
                        "When a file already exists",
                        SettingField::dropdown(
                            options(&[
                                ("ask", "Ask"),
                                ("skip", "Always skip"),
                                ("overwrite", "Always overwrite"),
                            ]),
                            |cx| match Prefs::get(cx).config.destination.on_conflict {
                                ConflictPref::Ask => "ask".into(),
                                ConflictPref::Skip => "skip".into(),
                                ConflictPref::Overwrite => "overwrite".into(),
                            },
                            |v, cx| {
                                let p = match v.as_ref() {
                                    "skip" => ConflictPref::Skip,
                                    "overwrite" => ConflictPref::Overwrite,
                                    _ => ConflictPref::Ask,
                                };
                                Prefs::update_config(cx, |c| c.destination.on_conflict = p)
                            },
                        ),
                    ))
                    .item(SettingItem::render(|_, _, cx| per_device_row(cx))),
            ),
        SettingPage::new("Conversion")
            .description("How photos and videos are made Android/PC friendly.")
            .group(
                SettingGroup::new()
                    .item(
                        SettingItem::new(
                            "JPEG quality for HEIC",
                            // Presets in a dropdown: simpler than a number box,
                            // and avoids a focus bug in the number input.
                            SettingField::dropdown(
                                [80u8, 85, 90, 92, 95, 98, 100]
                                    .iter()
                                    .map(|q| {
                                        let label = if *q == 92 {
                                            "92 (default)".to_owned()
                                        } else {
                                            q.to_string()
                                        };
                                        (
                                            SharedString::from(q.to_string()),
                                            SharedString::from(label),
                                        )
                                    })
                                    .collect(),
                                |cx| {
                                    Prefs::get(cx)
                                        .config
                                        .conversion
                                        .jpeg_quality
                                        .to_string()
                                        .into()
                                },
                                |v, cx| {
                                    if let Ok(q) = v.parse::<u8>() {
                                        Prefs::update_config(cx, |c| c.conversion.jpeg_quality = q)
                                    }
                                },
                            ),
                        )
                        .description("92 keeps detail at about half the size of 100."),
                    )
                    .item(
                        SettingItem::new(
                            "Videos",
                            SettingField::dropdown(
                                options(&[
                                    ("lossless", "Lossless MP4"),
                                    ("h264", "H.264 (slower)"),
                                ]),
                                |cx| match Prefs::get(cx).config.conversion.video {
                                    VideoPref::Lossless => "lossless".into(),
                                    VideoPref::H264 => "h264".into(),
                                },
                                |v, cx| {
                                    let p = if v == "h264" {
                                        VideoPref::H264
                                    } else {
                                        VideoPref::Lossless
                                    };
                                    Prefs::update_config(cx, |c| c.conversion.video = p)
                                },
                            ),
                        )
                        .description(
                            "Lossless keeps the iPhone's HDR video as is (plays in VLC, Android, \
                             iPhone). H.264 converts to standard colour and is much slower.",
                        ),
                    ),
            ),
        SettingPage::new("Library").group(
            SettingGroup::new()
                .item(
                    SettingItem::new(
                        "Show Hidden album",
                        SettingField::switch(
                            |cx| Prefs::get(cx).config.library.show_hidden,
                            |v, cx| Prefs::update_config(cx, |c| c.library.show_hidden = v),
                        ),
                    )
                    .description(
                        "The iPhone locks this album with Face ID or the passcode. When on, \
                         hidden items are listed under Hidden, never in All media.",
                    ),
                )
                .item(
                    SettingItem::new(
                        "Show Recently Deleted",
                        SettingField::switch(
                            |cx| Prefs::get(cx).config.library.show_recently_deleted,
                            |v, cx| {
                                Prefs::update_config(cx, |c| c.library.show_recently_deleted = v)
                            },
                        ),
                    )
                    .description("Listed greyed out in the library. Never copied."),
                ),
        ),
        SettingPage::new("App").group(
            SettingGroup::new()
                .item(SettingItem::new(
                    "Theme",
                    SettingField::dropdown(
                        options(&[("system", "System"), ("light", "Light"), ("dark", "Dark")]),
                        |cx| match Prefs::get(cx).config.app.theme {
                            ThemePref::System => "system".into(),
                            ThemePref::Light => "light".into(),
                            ThemePref::Dark => "dark".into(),
                        },
                        |v, cx| {
                            let t = match v.as_ref() {
                                "light" => ThemePref::Light,
                                "dark" => ThemePref::Dark,
                                _ => ThemePref::System,
                            };
                            Prefs::update_config(cx, |c| c.app.theme = t)
                        },
                    ),
                ))
                .item(
                    SettingItem::new(
                        "Check for updates on startup",
                        SettingField::switch(
                            |cx| Prefs::get(cx).config.app.check_updates_on_startup,
                            |v, cx| {
                                Prefs::update_config(cx, |c| c.app.check_updates_on_startup = v)
                            },
                        ),
                    )
                    .description("At most once a day. Only shows something when an update exists."),
                )
                .item(
                    SettingItem::new(
                        "Keep running in tray while copying",
                        SettingField::switch(
                            |cx| Prefs::get(cx).config.app.keep_in_tray,
                            |v, cx| Prefs::update_config(cx, |c| c.app.keep_in_tray = v),
                        ),
                    )
                    .description(
                        "Closing the window during a copy keeps Rollcopy in the tray, and it \
                         stays there after the copies finish. Quit from the tray menu.",
                    ),
                )
                .item(
                    SettingItem::new(
                        "Notify about transfers",
                        SettingField::switch(
                            |cx| Prefs::get(cx).config.app.notify_when_done,
                            |v, cx| Prefs::update_config(cx, |c| c.app.notify_when_done = v),
                        ),
                    )
                    .description(
                        "When copies finish, pause or resume, and Rollcopy isn't the active window.",
                    ),
                )
                .item(SettingItem::render(|_, _, cx| reset_row(cx)).description(
                    "Back to defaults: preferences, window layout, error history and \
                             the thumbnail cache. Your phone and copied files aren't touched.",
                )),
        ),
    ]
}

/// Base folder: current path, "Choose…" and "Reset".
fn base_folder_row(cx: &mut App) -> AnyElement {
    let theme = cx.theme().clone();
    let base = Prefs::get(cx).config.destination.base_folder.clone();
    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(div().font_weight(FontWeight::MEDIUM).child("Base folder"))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(base.display().to_string()),
        )
        .child(
            btn("base-choose")
                .small()
                .outline()
                .label("Choose…")
                .on_click(|_, _, cx| {
                    let rx = cx.prompt_for_paths(PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: Some("Use this folder".into()),
                    });
                    cx.spawn(async move |cx| {
                        if let Ok(Ok(Some(paths))) = rx.await
                            && let Some(dir) = paths.into_iter().next()
                        {
                            cx.update(|cx| {
                                Prefs::update_config(cx, |c| c.destination.base_folder = dir)
                            });
                        }
                    })
                    .detach();
                }),
        )
        .child(
            btn("base-reset")
                .small()
                .ghost()
                .label("Reset")
                .on_click(|_, _, cx| {
                    Prefs::update_config(cx, |c| c.destination.base_folder = default_base_folder())
                }),
        )
        .into_any_element()
}

/// Folders picked per phone with the toolbar's 📁, with a way to forget them.
fn per_device_row(cx: &mut App) -> AnyElement {
    let theme = cx.theme().clone();
    let devices = Prefs::get(cx).config.destination.devices.clone();
    let mut col = v_flex().w_full().gap_1().child(
        div()
            .font_weight(FontWeight::MEDIUM)
            .child("Folders chosen per phone"),
    );
    if devices.is_empty() {
        col = col.child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("None. Use 📁 in the toolbar to pick a folder for the connected phone."),
        );
    }
    for (i, (udid, path)) in devices.into_iter().enumerate() {
        let short = udid.chars().take(8).collect::<String>();
        col = col.child(
            h_flex()
                .gap_2()
                .items_center()
                .text_sm()
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("{short}…")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(path.display().to_string()),
                )
                .child(
                    btn(("forget-device", i))
                        .xsmall()
                        .ghost()
                        .label("Use default")
                        .on_click(move |_, _, cx| {
                            let u = udid.clone();
                            Prefs::update_config(cx, |c| {
                                c.destination.devices.remove(&u);
                            })
                        }),
                ),
        );
    }
    col.into_any_element()
}

/// Two-step "Reset and restart" (state kept in a global so the settings
/// item, which only sees `App`, can show the confirmation).
#[derive(Default)]
struct ResetPrompt(bool);

impl Global for ResetPrompt {}

fn reset_row(cx: &mut App) -> AnyElement {
    let theme = cx.theme().clone();
    let asking = cx.try_global::<ResetPrompt>().is_some_and(|r| r.0);
    let row = h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child("Reset Rollcopy"),
        )
        .child(div().flex_1());
    if !asking {
        return row
            .child(
                btn("reset-ask")
                    .small()
                    .outline()
                    .label("Reset and restart…")
                    .on_click(|_, _, cx| {
                        cx.set_global(ResetPrompt(true));
                        cx.refresh_windows();
                    }),
            )
            .into_any_element();
    }
    row.child(
        div()
            .text_sm()
            .text_color(theme.warning)
            .child("Copies in progress will stop."),
    )
    .child(
        btn("reset-cancel")
            .small()
            .ghost()
            .label("Cancel")
            .on_click(|_, _, cx| {
                cx.set_global(ResetPrompt(false));
                cx.refresh_windows();
            }),
    )
    .child(
        btn("reset-confirm")
            .small()
            .danger()
            .label("Reset and restart")
            .on_click(|_, _, cx| Prefs::reset_all_and_restart(cx)),
    )
    .into_any_element()
}

impl Render for PrefsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let notice = Prefs::get(cx).notice.clone();
        let path = Prefs::get(cx).config_path().display().to_string();
        let settings =
            RenderOnce::render(Settings::new("rollcopy-prefs").pages(pages()), window, cx)
                .into_any_element();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when_some(notice, |d, n| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .bg(theme.warning.opacity(0.15))
                        .child(n),
                )
            })
            .child(div().flex_1().min_h_0().child(settings))
            .child(
                div()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("Saved automatically to {path}")),
            )
    }
}
