//! Main window (PLAN §6): toolbar, left pane with device card and library
//! tree, centre pane with the connection guide or the selected items.

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use unspool_device::{DeviceInfo, NoDeviceHint, Phase};
use unspool_library::{Asset, Library, MediaKind, Source};

use crate::tree::{LibraryTree, Selection, month_name};
use crate::worker::Event;

const INSTALL_USBMUXD: &str = "sudo apt install usbmuxd";
/// Rows shown in the interim item list (the M3 grid virtualises properly).
const LIST_LIMIT: usize = 500;

enum LibState {
    None,
    Loading,
    Loaded { library: Library, tree: LibraryTree },
    Failed(String),
}

pub struct MainView {
    phase: Phase,
    library: LibState,
    selection: Selection,
    expanded: HashSet<i32>,
    show_details: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    pub fn new(
        events: async_channel::Receiver<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn(async move |this, cx| {
            while let Ok(ev) = events.recv().await {
                let alive = this.update(cx, |view, cx| {
                    view.on_event(ev);
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let appearance = cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
        });
        Self {
            phase: Phase::NoDevice { hint: None },
            library: LibState::None,
            selection: Selection::All,
            expanded: HashSet::new(),
            show_details: false,
            _tasks: vec![task],
            _subscriptions: vec![appearance],
        }
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Phase(p) => {
                eprintln!("phase: {}", phase_label(&p));
                let same_phone = p.udid().is_some() && p.udid() == self.phase.udid();
                if !matches!(p, Phase::Ready { .. }) || !same_phone {
                    self.library = LibState::None;
                    self.selection = Selection::All;
                    self.show_details = false;
                }
                self.phase = p;
            }
            Event::LibraryLoading => self.library = LibState::Loading,
            Event::Library(Ok(library)) => {
                let tree = LibraryTree::build(&library.assets);
                self.expanded.clear();
                if let Some(newest) = tree.years.first() {
                    self.expanded.insert(newest.year);
                }
                self.library = LibState::Loaded { library, tree };
            }
            Event::Library(Err(e)) => self.library = LibState::Failed(e),
        }
    }

    fn select(&mut self, sel: Selection) {
        if let Selection::Year(y) = sel
            && self.selection == sel
            && !self.expanded.remove(&y)
        {
            self.expanded.insert(y);
        } else if let Selection::Year(y) | Selection::Month(y, _) = sel {
            self.expanded.insert(y);
        }
        self.selection = sel;
    }

    // ---- toolbar -------------------------------------------------------

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status = match &self.phase {
            Phase::Ready { info } => info.name.clone(),
            Phase::NoDevice { .. } | Phase::UsbmuxdMissing => "No iPhone connected".into(),
            _ => "Connecting…".into(),
        };
        h_flex()
            .w_full()
            .px_4()
            .py_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(Icon::new(IconName::Smartphone).small())
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Unspool"))
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(status),
            )
    }

    // ---- left pane ---------------------------------------------------

    fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut pane = v_flex()
            .id("left-pane")
            .w(px(270.))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .p_3()
            .gap_3()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(section_label("DEVICES", &theme));

        pane = match &self.phase {
            Phase::Ready { info } => pane.child(self.render_device_card(info, cx)),
            Phase::NoDevice { .. } | Phase::UsbmuxdMissing => pane.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("No iPhone connected"),
            ),
            _ => pane.child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(Spinner::new().small())
                    .child("Connecting…"),
            ),
        };

        if let LibState::Loaded { tree, .. } = &self.library {
            pane = pane
                .child(section_label("LIBRARY", &theme))
                .child(self.render_tree(tree, cx));
        }
        pane
    }

    fn render_device_card(&self, info: &DeviceInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut card = v_flex()
            .gap_1()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::Smartphone).small())
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(info.name.clone()),
                    ),
            )
            .child(muted(info.model.clone(), &theme))
            .child(muted(format!("iOS {}", info.ios_version), &theme));

        if let (Some(total), Some(free)) = (info.storage_total, info.storage_free) {
            let used = total.saturating_sub(free);
            let frac = if total > 0 {
                used as f32 / total as f32
            } else {
                0.
            };
            card = card
                .child(
                    div()
                        .mt_1()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(theme.muted)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .w(relative(frac.clamp(0., 1.)))
                                .bg(theme.primary),
                        ),
                )
                .child(muted(format!("{} used of {}", gb(used), gb(total)), &theme));
        }
        if let Some(pct) = info.battery_percent {
            let charging = info.battery_charging == Some(true);
            card = card.child(
                h_flex()
                    .gap_1()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(
                        Icon::new(if charging {
                            IconName::BatteryCharging
                        } else {
                            IconName::Battery
                        })
                        .xsmall(),
                    )
                    .child(if charging {
                        format!("{pct}% · charging")
                    } else {
                        format!("{pct}%")
                    }),
            );
        }

        let show = self.show_details;
        card = card.child(
            div()
                .id("device-details")
                .mt_1()
                .text_xs()
                .text_color(theme.link)
                .cursor_pointer()
                .child(if show { "Hide details" } else { "Details" })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_details = !this.show_details;
                    cx.notify();
                })),
        );
        if show {
            let backup = match info.icloud_backup {
                Some(true) => "on",
                Some(false) => "off",
                None => "unknown",
            };
            card = card
                .child(detail(
                    "Serial",
                    info.serial_number.as_deref().unwrap_or("-"),
                    &theme,
                ))
                .child(detail("UDID", &info.udid, &theme))
                .child(detail("iCloud Backup", backup, &theme));
        }
        card
    }

    fn render_tree(&self, tree: &LibraryTree, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex().gap_0p5().child(self.tree_row(
            "all".into(),
            None,
            "All media".into(),
            tree.all,
            Selection::All,
            false,
            cx,
        ));
        for y in &tree.years {
            let open = self.expanded.contains(&y.year);
            list = list.child(self.tree_row(
                format!("y{}", y.year).into(),
                Some(open),
                y.year.to_string().into(),
                y.count,
                Selection::Year(y.year),
                false,
                cx,
            ));
            if open {
                for (m, n) in &y.months {
                    list = list.child(div().pl_5().child(self.tree_row(
                        format!("m{}-{m}", y.year).into(),
                        None,
                        month_name(*m).into(),
                        *n,
                        Selection::Month(y.year, *m),
                        false,
                        cx,
                    )));
                }
            }
        }
        if tree.undated > 0 {
            list = list.child(self.tree_row(
                "undated".into(),
                None,
                "Undated".into(),
                tree.undated,
                Selection::Undated,
                false,
                cx,
            ));
        }
        // Greyed out, never copied (PLAN §6).
        list.child(self.tree_row(
            "deleted".into(),
            None,
            "Recently Deleted".into(),
            tree.recently_deleted,
            Selection::RecentlyDeleted,
            true,
            cx,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn tree_row(
        &self,
        id: SharedString,
        expanded: Option<bool>,
        label: SharedString,
        count: usize,
        sel: Selection,
        greyed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected = self.selection == sel;
        h_flex()
            .id(id)
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.accent))
            .when(!selected, |d| d.hover(|s| s.bg(theme.muted)))
            .when(greyed, |d| d.text_color(theme.muted_foreground))
            .child(match expanded {
                Some(true) => Icon::new(IconName::ChevronDown).xsmall().into_any_element(),
                Some(false) => Icon::new(IconName::ChevronRight)
                    .xsmall()
                    .into_any_element(),
                None => div().w(px(12.)).into_any_element(),
            })
            .child(div().flex_1().child(label))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(count.to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(sel);
                cx.notify();
            }))
    }

    // ---- centre pane -------------------------------------------------

    fn render_center(&self, cx: &mut Context<Self>) -> AnyElement {
        match (&self.phase, &self.library) {
            (Phase::Ready { .. }, LibState::Loaded { library, .. }) => {
                self.render_items(library, cx).into_any_element()
            }
            (Phase::Ready { .. }, LibState::Failed(e)) => guide(
                IconName::TriangleAlert,
                "Couldn't read the Photos library",
                e.clone(),
                true,
                cx,
            ),
            (Phase::Ready { .. }, _) => guide_busy(
                IconName::Images,
                "Reading your Photos library…",
                "This takes a few seconds.",
                cx,
            ),
            (phase, _) => render_guide(phase, cx),
        }
    }

    fn render_items(&self, library: &Library, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let items: Vec<&Asset> = library
            .assets
            .iter()
            .filter(|a| LibraryTree::contains(self.selection, a))
            .collect();
        let title = match self.selection {
            Selection::All => "All media".to_owned(),
            Selection::Year(y) => y.to_string(),
            Selection::Month(y, m) => format!("{} {y}", month_name(m)),
            Selection::Undated => "Undated".to_owned(),
            Selection::RecentlyDeleted => "Recently Deleted".to_owned(),
        };
        let photos = items.iter().filter(|a| a.kind == MediaKind::Photo).count();
        let mut summary = format!(
            "{} items · {photos} photos · {} videos",
            items.len(),
            items.len() - photos
        );
        if let Source::DcimScan { reason } = &library.source {
            summary.push_str(&format!(" · from DCIM folder ({reason})"));
        }

        let mut rows = v_flex().gap_0p5();
        for a in items.iter().rev().take(LIST_LIMIT) {
            rows = rows.child(item_row(a, &theme));
        }
        if items.len() > LIST_LIMIT {
            rows = rows.child(muted(
                format!("… and {} more", items.len() - LIST_LIMIT),
                &theme,
            ));
        }

        v_flex()
            .id("items")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .p_4()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(muted(summary, &theme))
                    .when(self.selection == Selection::RecentlyDeleted, |d| {
                        d.child(muted(
                            "Items in Recently Deleted are shown for reference and never copied."
                                .to_owned(),
                            &theme,
                        ))
                    }),
            )
            .child(rows)
    }
}

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_toolbar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.render_left(cx))
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(self.render_center(cx)),
                    ),
            )
    }
}

// ---- connection guide (PLAN §5) ---------------------------------------

fn render_guide(phase: &Phase, cx: &mut Context<MainView>) -> AnyElement {
    match phase {
        Phase::UsbmuxdMissing => {
            let theme = cx.theme().clone();
            guide_with(
                IconName::Terminal,
                "Install usbmuxd",
                "Unspool needs the usbmuxd service to talk to your iPhone. Install it, then plug your iPhone in.",
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
                        Button::new("copy-install")
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
            "Then enter your passcode. This lets Unspool read your photos and videos.",
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
            "Unspool continues as soon as it's unlocked.",
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

fn guide(
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

fn guide_busy(icon: IconName, title: &str, body: &str, cx: &mut Context<MainView>) -> AnyElement {
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

fn guide_with(
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

fn guide_frame(
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

// ---- small helpers ------------------------------------------------------

fn section_label(text: &'static str, theme: &Theme) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(text)
}

fn muted(text: String, theme: &Theme) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(text)
}

fn detail(label: &str, value: &str, theme: &Theme) -> impl IntoElement {
    v_flex()
        .text_xs()
        .child(
            div()
                .text_color(theme.muted_foreground)
                .child(label.to_owned()),
        )
        .child(div().child(value.to_owned()))
}

fn gb(bytes: u64) -> String {
    format!("{:.0} GB", bytes as f64 / 1e9)
}

fn item_row(a: &Asset, theme: &Theme) -> impl IntoElement {
    let when = a
        .captured_local()
        .map(|d| d.format("%-d %b %Y %H:%M").to_string())
        .unwrap_or_else(|| "-".into());
    let mut badges = Vec::new();
    if a.kind == MediaKind::Video {
        badges.push(match a.duration_secs {
            Some(d) => format!("▶ {}:{:02}", d as u64 / 60, d as u64 % 60),
            None => "▶".into(),
        });
    }
    if a.live_photo {
        badges.push("Live".into());
    }
    if a.is_edited() {
        badges.push("Edited".into());
    }
    if a.favorite {
        badges.push("♥".into());
    }
    h_flex()
        .w_full()
        .gap_3()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .hover(|s| s.bg(theme.muted))
        .child(
            Icon::new(if a.kind == MediaKind::Video {
                IconName::Video
            } else {
                IconName::Image
            })
            .xsmall()
            .text_color(theme.muted_foreground),
        )
        .child(div().w(px(150.)).child(a.filename.clone()))
        .child(
            div()
                .w(px(150.))
                .text_color(theme.muted_foreground)
                .child(when),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(badges.join(" · ")),
        )
}

/// Short phase name for the log (stderr).
fn phase_label(p: &Phase) -> &'static str {
    match p {
        Phase::UsbmuxdMissing => "usbmuxd-missing",
        Phase::NoDevice { hint: None } => "no-device",
        Phase::NoDevice {
            hint: Some(NoDeviceHint::Flaky),
        } => "no-device-flaky",
        Phase::NoDevice {
            hint: Some(NoDeviceHint::AppleUsbNotListed),
        } => "no-device-not-listed",
        Phase::Connecting { .. } => "connecting",
        Phase::WaitingForTrust { .. } => "waiting-for-trust",
        Phase::TrustDenied { .. } => "trust-denied",
        Phase::Locked { .. } => "locked",
        Phase::Ready { .. } => "ready",
        Phase::Failed { .. } => "failed",
    }
}
