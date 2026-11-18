//! Left pane, in sections: Library (All Media), Device (the phone, then its
//! years and months), Albums, and Collections (Hidden, Recently Deleted).
//! Compact rows with a tinted selection; thin headings, no cards.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_device::{DeviceInfo, Phase};

use crate::palette::RADIUS;
use crate::tree::{LibraryTree, Selection, month_name};

use crate::view::*;
use crate::widgets::*;

/// Height of a sidebar row.
const ROW_H: f32 = 26.;
/// Left inset of a nested row (months, albums): lines its text up with
/// the parent's label.
const NESTED: f32 = 22.;

/// A sidebar section heading ("LIBRARY").
fn heading(text: &'static str, theme: &Theme) -> Div {
    h_flex()
        .w_full()
        .h(px(ROW_H))
        .px_2()
        .mt_2()
        .items_center()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.muted_foreground)
        .child(div().flex_1().child(text))
}

impl MainView {
    pub(crate) fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let tree = match &self.library {
            LibState::Loaded { tree, .. } => Some(tree),
            _ => None,
        };
        let mut pane = v_flex().w_full().px_2().pb_3();

        if let Some(tree) = tree {
            pane = pane.child(heading("LIBRARY", &theme)).child(self.tree_row(
                "all".into(),
                None,
                Some(IconName::Images),
                "All Media".into(),
                tree.all,
                Selection::All,
                false,
                cx,
            ));
        }

        pane = pane.child(heading("DEVICE", &theme));
        pane = match &self.phase {
            Phase::Ready { info } => pane.child(self.render_device_card(info, cx)),
            Phase::NoDevice { .. } | Phase::UsbmuxdMissing => pane.child(
                h_flex()
                    .h(px(ROW_H))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Smartphone).small())
                    .child("No iPhone connected"),
            ),
            _ => pane.child(
                h_flex()
                    .h(px(ROW_H))
                    .px_2()
                    .gap_2()
                    .text_sm()
                    .child(Spinner::new().small())
                    .child("Connecting…"),
            ),
        };

        if let Some(tree) = tree {
            pane = self.render_tree(pane, tree, cx);
        }
        v_flex()
            .id("left-pane")
            .w(px(LEFT_W))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scrollbar()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .child(pane)
    }

    /// The phone: name and battery, model and iOS, a storage bar, and
    /// Details (serial, UDID, backup) on request.
    pub(crate) fn render_device_card(
        &self,
        info: &DeviceInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let battery = info.battery_percent.map(|pct| {
            let charging = info.battery_charging == Some(true);
            h_flex()
                .flex_shrink_0()
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Icon::new(if charging {
                        IconName::BatteryCharging
                    } else {
                        IconName::Battery
                    })
                    .xsmall(),
                )
                .child(format!("{pct}%"))
        });
        let mut card = v_flex()
            .w_full()
            .px_2()
            .pb_1()
            .gap_1()
            .child(
                h_flex()
                    .h(px(ROW_H))
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Smartphone)
                            .small()
                            .text_color(theme.link),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(info.name.clone()),
                    )
                    .children(battery),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} · iOS {}", info.model, info.ios_version)),
            );

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
                        .h(px(4.))
                        .rounded_full()
                        .bg(theme.border)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .w(relative(frac.clamp(0., 1.)))
                                .bg(theme.primary),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("{} free of {}", gb(free), gb(total))),
                );
        }

        let show = self.show_details;
        card = card.child(
            h_flex()
                .id("device-details")
                .text_xs()
                .text_color(theme.link)
                .cursor_pointer()
                .gap_1()
                .child(
                    Icon::new(if show {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall(),
                )
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

    /// The phone's years and months, then the Albums and Collections
    /// sections, added to `pane`.
    pub(crate) fn render_tree(
        &self,
        mut pane: Div,
        tree: &LibraryTree,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme().clone();
        for y in &tree.years {
            let open = self.expanded.contains(&y.year);
            pane = pane.child(self.tree_row(
                format!("y{}", y.year).into(),
                Some(open),
                None,
                y.year.to_string().into(),
                y.count,
                Selection::Year(y.year),
                false,
                cx,
            ));
            if open {
                for (m, n) in &y.months {
                    pane = pane.child(div().pl(px(NESTED)).child(self.tree_row(
                        format!("m{}-{m}", y.year).into(),
                        None,
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
            pane = pane.child(self.tree_row(
                "undated".into(),
                None,
                Some(IconName::CalendarX),
                "Undated".into(),
                tree.undated,
                Selection::Undated,
                false,
                cx,
            ));
        }

        if !tree.albums.is_empty() {
            let open = self.albums_open;
            pane = pane.child(self.albums_header(tree.albums.len(), open, cx));
            if open {
                for (i, (name, n)) in tree.albums.iter().enumerate() {
                    pane = pane.child(self.tree_row(
                        format!("album{i}").into(),
                        None,
                        Some(IconName::BookImage),
                        name.clone().into(),
                        *n,
                        Selection::Album(i),
                        false,
                        cx,
                    ));
                }
            }
        }

        let hidden = self.config.library.show_hidden && tree.hidden > 0;
        let deleted = self.config.library.show_recently_deleted;
        if hidden || deleted {
            pane = pane.child(heading("COLLECTIONS", &theme));
        }
        if hidden {
            pane = pane.child(self.tree_row(
                "hidden".into(),
                None,
                Some(IconName::EyeOff),
                "Hidden".into(),
                tree.hidden,
                Selection::Hidden,
                false,
                cx,
            ));
        }
        // Greyed out, never copied (PLAN §6); can be hidden in Preferences.
        if deleted {
            pane = pane.child(self.tree_row(
                "deleted".into(),
                None,
                Some(IconName::Trash),
                "Recently Deleted".into(),
                tree.recently_deleted,
                Selection::RecentlyDeleted,
                true,
                cx,
            ));
        }
        pane
    }

    /// The ALBUMS heading: opens and closes the list of albums (not a
    /// selection).
    pub(crate) fn albums_header(
        &self,
        count: usize,
        open: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        heading("ALBUMS", &theme)
            .id("albums")
            .gap_1()
            .rounded(px(RADIUS))
            .cursor_pointer()
            .hover(|s| s.bg(theme.muted))
            .child(count.to_string())
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall(),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.albums_open = !this.albums_open;
                cx.notify();
            }))
    }

    /// One row: a chevron (years) or an icon, the label, and a count.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tree_row(
        &self,
        id: SharedString,
        expanded: Option<bool>,
        icon: Option<IconName>,
        label: SharedString,
        count: usize,
        sel: Selection,
        greyed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected = self.selection == sel;
        let icon_color = if selected {
            theme.link
        } else {
            theme.muted_foreground
        };
        let lead = match (expanded, sel) {
            // The chevron toggles on its own, whatever is selected.
            (Some(open), Selection::Year(year)) => div()
                .id(SharedString::from(format!("chev-{year}")))
                .flex()
                .items_center()
                .justify_center()
                .size(px(16.))
                .rounded(px(RADIUS))
                .cursor_pointer()
                .text_color(icon_color)
                .hover(|s| s.bg(theme.muted))
                .child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall(),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if !this.expanded.remove(&year) {
                        this.expanded.insert(year);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }))
                .into_any_element(),
            _ => div()
                .flex()
                .items_center()
                .w(px(16.))
                .flex_shrink_0()
                .children(icon.map(|i| Icon::new(i).small().text_color(icon_color)))
                .into_any_element(),
        };
        h_flex()
            .id(id)
            .w_full()
            .h(px(ROW_H))
            .px_2()
            .gap_2()
            .items_center()
            .rounded(px(RADIUS))
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.accent))
            .when(!selected, |d| d.hover(|s| s.bg(theme.muted)))
            .when(greyed, |d| d.text_color(theme.muted_foreground))
            .child(lead)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(label),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(count.to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select(sel);
                cx.notify();
            }))
    }
}
