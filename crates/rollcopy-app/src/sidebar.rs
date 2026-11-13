//! Left pane: the device card and the library tree (years, albums,
//! Hidden, Recently Deleted).

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_device::{DeviceInfo, Phase};

use crate::tree::{LibraryTree, Selection, month_name};

use crate::view::*;
use crate::widgets::*;

impl MainView {
    pub(crate) fn render_left(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut pane = v_flex()
            .id("left-pane")
            .w(px(LEFT_W))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scrollbar()
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

    pub(crate) fn render_device_card(
        &self,
        info: &DeviceInfo,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
                .flex()
                .items_center()
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

    pub(crate) fn render_tree(
        &self,
        tree: &LibraryTree,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
        if !tree.albums.is_empty() {
            let open = self.albums_open;
            list = list.child(self.albums_header(tree.albums.len(), open, cx));
            if open {
                for (i, (name, n)) in tree.albums.iter().enumerate() {
                    list = list.child(div().pl_5().child(self.tree_row(
                        format!("album{i}").into(),
                        None,
                        name.clone().into(),
                        *n,
                        Selection::Album(i),
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
        if self.config.library.show_hidden && tree.hidden > 0 {
            list = list.child(self.tree_row(
                "hidden".into(),
                None,
                "Hidden".into(),
                tree.hidden,
                Selection::Hidden,
                false,
                cx,
            ));
        }
        // Greyed out, never copied (PLAN §6); can be hidden in Preferences.
        if self.config.library.show_recently_deleted {
            list = list.child(self.tree_row(
                "deleted".into(),
                None,
                "Recently Deleted".into(),
                tree.recently_deleted,
                Selection::RecentlyDeleted,
                true,
                cx,
            ));
        }
        list
    }

    /// "Albums": opens and closes the list of albums (not a selection).
    pub(crate) fn albums_header(
        &self,
        count: usize,
        open: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        h_flex()
            .id("albums")
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme.muted))
            .child(
                div().w(px(16.)).child(
                    Icon::new(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .xsmall(),
                ),
            )
            .child(div().flex_1().child("Albums"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(count.to_string()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.albums_open = !this.albums_open;
                cx.notify();
            }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tree_row(
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
            .child(match (expanded, sel) {
                // The chevron toggles on its own, whatever is selected.
                (Some(open), Selection::Year(year)) => div()
                    .id(SharedString::from(format!("chev-{year}")))
                    .p_0p5()
                    .rounded_sm()
                    .cursor_pointer()
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
                (_, Selection::Hidden) => div()
                    .w(px(16.))
                    .child(Icon::new(IconName::EyeOff).xsmall())
                    .into_any_element(),
                (_, Selection::RecentlyDeleted) => div()
                    .w(px(16.))
                    .child(Icon::new(IconName::Trash).xsmall())
                    .into_any_element(),
                _ => div().w(px(16.)).into_any_element(),
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
}
