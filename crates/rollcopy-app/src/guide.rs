//! Connection guide (PLAN §5): what to do when no phone is ready.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_device::{NoDeviceHint, Phase};

use crate::view::*;
use crate::widgets::*;

const INSTALL_USBMUXD: &str = "sudo apt install usbmuxd";

pub(crate) fn render_guide(phase: &Phase, cx: &mut Context<MainView>) -> AnyElement {
    match phase {
        Phase::UsbmuxdMissing => {
            let theme = cx.theme().clone();
            guide_with(
                IconName::Terminal,
                "Install usbmuxd",
                "Rollcopy needs the usbmuxd service to talk to your iPhone. Install it, then plug your iPhone in.",
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(theme.muted)
                            .font_family("monospace")
                            .text_sm()
                            .child(INSTALL_USBMUXD),
                    )
                    .child(
                        btn("copy-install")
                            .small()
                            .outline()
                            .icon(IconName::Copy)
                            .label("Copy")
                            .on_click(|_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    INSTALL_USBMUXD.to_owned(),
                                ))
                            }),
                    )
                    .into_any_element(),
                cx,
            )
        }
        Phase::NoDevice { hint: None } => guide(
            IconName::Plug,
            "Plug in your iPhone with a cable",
            "Use a cable that carries data, connected straight to this computer.".into(),
            false,
            cx,
        ),
        Phase::NoDevice {
            hint: Some(NoDeviceHint::Flaky),
        } => guide(
            IconName::Unplug,
            "The connection keeps dropping",
            "Your iPhone connected for a moment and then disconnected. Try another cable or another USB port, without a hub."
                .into(),
            true,
            cx,
        ),
        Phase::NoDevice {
            hint: Some(NoDeviceHint::AppleUsbNotListed),
        } => guide(
            IconName::Usb,
            "Your iPhone isn't responding",
            "It's plugged in but not answering. Unplug it and plug it back in, or try another cable or USB port."
                .into(),
            true,
            cx,
        ),
        Phase::Connecting { .. } => guide_busy(
            IconName::Smartphone,
            "Connecting to your iPhone…",
            "",
            cx,
        ),
        Phase::WaitingForTrust { .. } => guide_busy(
            IconName::ShieldCheck,
            "Unlock your iPhone and tap Trust",
            "Then enter your passcode. This lets Rollcopy read your photos and videos.",
            cx,
        ),
        Phase::TrustDenied { .. } => guide(
            IconName::ShieldX,
            "Your iPhone said Don't Trust",
            "Unplug your iPhone and plug it back in to be asked again.".into(),
            true,
            cx,
        ),
        Phase::Locked { .. } => guide_busy(
            IconName::Lock,
            "Unlock your iPhone",
            "Rollcopy continues as soon as it's unlocked.",
            cx,
        ),
        Phase::Failed { message, .. } => guide_busy(
            IconName::TriangleAlert,
            "Couldn't connect to your iPhone",
            message,
            cx,
        ),
        // Ready is handled by the caller.
        Phase::Ready { .. } => div().into_any_element(),
    }
}

pub(crate) fn guide(
    icon: IconName,
    title: &str,
    body: String,
    warn: bool,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    guide_frame(
        Icon::new(icon)
            .with_size(px(48.))
            .text_color(if warn {
                theme.warning
            } else {
                theme.muted_foreground
            })
            .into_any_element(),
        title,
        &body,
        None,
        cx,
    )
}

pub(crate) fn guide_busy(
    icon: IconName,
    title: &str,
    body: &str,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    guide_frame(
        Icon::new(icon)
            .with_size(px(48.))
            .text_color(theme.muted_foreground)
            .into_any_element(),
        title,
        body,
        Some(Spinner::new().into_any_element()),
        cx,
    )
}

pub(crate) fn guide_with(
    icon: IconName,
    title: &str,
    body: &str,
    extra: AnyElement,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    guide_frame(
        Icon::new(icon)
            .with_size(px(48.))
            .text_color(theme.muted_foreground)
            .into_any_element(),
        title,
        body,
        Some(extra),
        cx,
    )
}

pub(crate) fn guide_frame(
    icon: AnyElement,
    title: &str,
    body: &str,
    extra: Option<AnyElement>,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let theme = cx.theme().clone();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_8()
        .child(icon)
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.to_owned()),
        )
        .when(!body.is_empty(), |d| {
            d.child(
                div()
                    .max_w(px(460.))
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(body.to_owned()),
            )
        })
        .children(extra)
        .into_any_element()
}
