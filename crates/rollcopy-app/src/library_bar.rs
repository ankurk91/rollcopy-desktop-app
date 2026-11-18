//! The library bar above the grid (title and count, filter toggles,
//! Copied / Not copied, sort, zoom, grid/list, info) and the empty state.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::Source;

use crate::filters::{Sort, SortKey};
use crate::tree::{Selection, month_name};
use crate::view::{Filter, LibState, MainView, SyncFilter, btn, muted};

use crate::grid::{PAD, TILE_MAX, TILE_MIN};

impl MainView {
    pub(crate) fn render_grid_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let copyable = self
            .visible
            .iter()
            .filter(|&&i| !self.assets()[i].trashed)
            .count();
        let counts = self.filter_counts;
        let count = |f: Filter| counts[f.count_index()];

        let title = match self.selection {
            Selection::All => "All Media".to_owned(),
            Selection::Year(y) => y.to_string(),
            Selection::Month(y, m) => format!("{} {y}", month_name(m)),
            Selection::Undated => "Undated".to_owned(),
            Selection::Hidden => "Hidden".to_owned(),
            Selection::RecentlyDeleted => "Recently Deleted".to_owned(),
            Selection::Album(i) => match &self.library {
                LibState::Loaded { tree, .. } => tree
                    .albums
                    .get(i)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default(),
                _ => String::new(),
            },
        };
        let n = self.visible.len();
        let mut summary = format!("{n} item{}", if n == 1 { "" } else { "s" });
        if !self.selected.is_empty() {
            summary.push_str(&format!(" · {} selected", self.selected.len()));
        }
        if let LibState::Loaded { library, .. } = &self.library
            && let Source::DcimScan { reason } = &library.source
        {
            summary.push_str(&format!(" · from DCIM folder ({reason})"));
        }

        // Type filters: a row of toggles with an icon and a count, named in
        // the tooltip. Empty ones are left out (unless on) to keep it short.
        let chips: Vec<AnyElement> = Filter::ALL
            .iter()
            .filter(|&&f| f == Filter::All || f == self.filter || count(f) > 0)
            .map(|&f| {
                let n = count(f);
                btn(("filter", f.count_index()))
                    .xsmall()
                    .ghost()
                    .selected(self.filter == f)
                    .icon(filter_icon(f))
                    .label(if f == Filter::All {
                        format!("All {n}")
                    } else {
                        n.to_string()
                    })
                    .tooltip(format!("{} ({n})", f.label()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.filter = f;
                        this.recompute_visible();
                        // Keep arrow keys on the grid, not on the toggle.
                        window.focus(&this.focus, cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect();

        let divider = || div().w(px(1.)).h(px(16.)).mx_1().bg(theme.border);

        v_flex()
            .flex_shrink_0()
            .px(px(PAD))
            .pt_2()
            .pb_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .h(px(32.))
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(muted(summary, &theme))
                    .child(div().flex_1())
                    .when(!self.visible.is_empty(), |d| {
                        d.child(
                            btn("select-all")
                                .xsmall()
                                .ghost()
                                .label("Select all")
                                .tooltip("Ctrl+A")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.select_all_visible();
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(!self.selected.is_empty(), |d| {
                        d.child(
                            btn("clear-selection")
                                .xsmall()
                                .ghost()
                                .label("Clear selection")
                                .tooltip("Esc")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.selected.clear();
                                    this.anchor = None;
                                    window.focus(&this.focus, cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .when(copyable > 0 && self.dest_root().is_some(), |d| {
                        d.child(
                            btn("copy-all")
                                .xsmall()
                                .outline()
                                .label(format!("Copy all ({copyable})"))
                                .tooltip("Copy everything shown here")
                                // A big action: ask first (Copy selected doesn't).
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.copy_all_prompt = true;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .items_center()
                    .children(chips)
                    .when(self.dest_root().is_some(), |d| {
                        d.child(divider())
                            .child(sync_chip(
                                "s-copied",
                                "Copied",
                                SyncFilter::Copied,
                                self.sync_filter,
                                self.sync_counts[1],
                                cx,
                            ))
                            .child(sync_chip(
                                "s-not-copied",
                                "Not copied",
                                SyncFilter::NotCopied,
                                self.sync_filter,
                                self.sync_counts[2],
                                cx,
                            ))
                    })
                    .child(div().flex_1())
                    .child(self.render_sort_button(cx))
                    .when(!self.list_view, |d| {
                        d.child(divider())
                            .child(
                                btn("zoom-out")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::ZoomOut)
                                    .tooltip("Smaller thumbnails (Ctrl+−)")
                                    .disabled(self.tile_target <= TILE_MIN)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.zoom(-1, cx);
                                        window.focus(&this.focus, cx);
                                    })),
                            )
                            .child(
                                btn("zoom-in")
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::ZoomIn)
                                    .tooltip("Bigger thumbnails (Ctrl+=, or Ctrl+scroll)")
                                    .disabled(self.tile_target >= TILE_MAX)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.zoom(1, cx);
                                        window.focus(&this.focus, cx);
                                    })),
                            )
                    })
                    .child(divider())
                    .child(
                        btn("view-grid")
                            .xsmall()
                            .ghost()
                            .selected(!self.list_view)
                            .icon(IconName::LayoutGrid)
                            .tooltip("Grid")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_list_view(false, cx);
                                window.focus(&this.focus, cx);
                            })),
                    )
                    .child(
                        btn("view-list")
                            .xsmall()
                            .ghost()
                            .selected(self.list_view)
                            .icon(IconName::List)
                            .tooltip("List (no thumbnails)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_list_view(true, cx);
                                window.focus(&this.focus, cx);
                            })),
                    )
                    .child(divider())
                    .child(
                        btn("toggle-info")
                            .xsmall()
                            .ghost()
                            .selected(self.preview_open)
                            .icon(IconName::PanelRight)
                            .tooltip(if self.preview_open {
                                "Hide info (I)"
                            } else {
                                "Show info (I)"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_info_open(!this.preview_open);
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    ),
            )
            .when(self.selection == Selection::RecentlyDeleted, |d| {
                d.child(muted(
                    "Items in Recently Deleted are shown for reference and are never copied."
                        .to_owned(),
                    &theme,
                ))
            })
    }

    /// The sort button and its menu: what to sort by, then which way.
    fn render_sort_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sort = self.sort;
        let view = cx.entity().downgrade();
        let set = move |next: Sort| {
            let view = view.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |this, cx| {
                    this.sort = next;
                    this.recompute_visible();
                    window.focus(&this.focus, cx);
                    cx.notify();
                });
            }
        };
        btn("sort")
            .xsmall()
            .ghost()
            .icon(if sort.descending {
                IconName::ArrowDownWideNarrow
            } else {
                IconName::ArrowUpNarrowWide
            })
            .label(sort.label())
            .tooltip(format!(
                "Sorted by {}, {}",
                sort.key.label().to_lowercase(),
                sort.direction().to_lowercase()
            ))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                for key in SortKey::ALL {
                    // A new key starts in its natural direction: newest,
                    // largest, A to Z.
                    let next = Sort {
                        key,
                        descending: matches!(key, SortKey::Date | SortKey::Size),
                    };
                    menu = menu.item(
                        PopupMenuItem::new(key.label())
                            .checked(sort.key == key)
                            .on_click(set(if sort.key == key { sort } else { next })),
                    );
                }
                let (desc, asc) = sort.direction_labels();
                menu.separator()
                    .item(
                        PopupMenuItem::new(desc)
                            .checked(sort.descending)
                            .on_click(set(Sort {
                                descending: true,
                                ..sort
                            })),
                    )
                    .item(
                        PopupMenuItem::new(asc)
                            .checked(!sort.descending)
                            .on_click(set(Sort {
                                descending: false,
                                ..sort
                            })),
                    )
            })
    }
}

/// Icon for a type filter chip.
fn filter_icon(f: Filter) -> IconName {
    match f {
        Filter::All => IconName::LayoutGrid,
        Filter::Photos => IconName::Image,
        Filter::Videos => IconName::Video,
        Filter::Edited => IconName::SlidersHorizontal,
        Filter::Favorites => IconName::Heart,
        Filter::Live => IconName::CircleDot,
        Filter::Screenshots => IconName::Scan,
        Filter::Selfies => IconName::UserRound,
    }
}

/// Copied / Not copied chip; clicking the active one turns the filter off.
fn sync_chip(
    id: &'static str,
    label: &str,
    f: SyncFilter,
    current: SyncFilter,
    n: usize,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    // `current` is passed in: reading the view here would panic mid-render.
    let active = current == f;
    btn(id)
        .xsmall()
        .ghost()
        .selected(active)
        .label(format!("{label} {n}"))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.sync_filter = if this.sync_filter == f {
                SyncFilter::All
            } else {
                f
            };
            this.recompute_visible();
            window.focus(&this.focus, cx);
            cx.notify();
        }))
}

impl MainView {
    /// Why the grid is empty, and the one action that helps.
    pub(crate) fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let searching = !crate::filters::Query::new(&self.query).is_empty();
        let filtered =
            searching || self.filter != Filter::All || self.sync_filter != SyncFilter::All;
        let show_hidden = self.config.library.show_hidden;
        let library_empty = !self
            .assets()
            .iter()
            .any(|a| !a.trashed && (show_hidden || !a.hidden));
        let (icon, title, body): (IconName, &str, String) = if library_empty {
            (
                IconName::Images,
                "No photos or videos on this iPhone",
                "Take a photo, or check that Photos isn't still syncing from iCloud.".into(),
            )
        } else if searching {
            (
                IconName::Search,
                "Nothing matches your search",
                format!(
                    "No item here has \"{}\" in its name, place, album, app or camera.",
                    self.query.trim()
                ),
            )
        } else if filtered {
            let what = match (self.filter, self.sync_filter) {
                (_, SyncFilter::Copied) => "Nothing here has been copied yet.",
                (_, SyncFilter::NotCopied) => "Everything here is already copied.",
                (Filter::Photos, _) => "No photos here.",
                (Filter::Videos, _) => "No videos here.",
                (Filter::Edited, _) => "No edited items here.",
                (Filter::Favorites, _) => "No favourites here.",
                (Filter::Live, _) => "No Live Photos here.",
                (Filter::Screenshots, _) => "No screenshots here.",
                (Filter::Selfies, _) => "No selfies here.",
                (Filter::All, SyncFilter::All) => "Nothing here.",
            };
            (
                IconName::Images,
                what,
                "Clear the filters to see everything.".into(),
            )
        } else if self.selection == Selection::RecentlyDeleted {
            (IconName::Trash, "Recently Deleted is empty", String::new())
        } else {
            (IconName::Images, "Nothing here", String::new())
        };
        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .child(crate::viewer::big_icon(icon, theme.muted_foreground))
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_owned()),
            )
            .when(!body.is_empty(), |d| {
                d.child(div().text_color(theme.muted_foreground).child(body))
            })
            .when(filtered && !library_empty, |d| {
                d.child(
                    btn("clear-filters")
                        .small()
                        .outline()
                        .label("Show everything")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.filter = Filter::All;
                            this.sync_filter = SyncFilter::All;
                            this.query.clear();
                            this.search.update(cx, |s, cx| s.set_value("", window, cx));
                            this.recompute_visible();
                            window.focus(&this.focus, cx);
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }
}
