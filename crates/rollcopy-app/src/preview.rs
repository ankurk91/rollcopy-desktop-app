//! Right pane: preview and info for the focused item (PLAN §6).
//!
//! The preview uses iOS's own 480×360 / 640×360 thumbnail, which is sharper
//! than the 300 px pane needs. Full-resolution viewing goes through "Open",
//! which hands the original (or edited version) to the system viewer.

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::{Asset, MediaKind};

use gpui_kit::component::resizable::{HandleEdge, resize_handle};

use crate::view::{MainView, ResizePreview, Thumb, btn, muted, section_label};

impl MainView {
    /// Fixed frame (width, border, resize handle) around the scrolling body.
    pub(crate) fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let body = self.render_preview_body(cx).into_any_element();
        v_flex()
            .relative()
            .w(px(self.preview_w))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .border_l_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(px(36.))
                    .px_3()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Info"),
                    )
                    .child(
                        btn("close-preview")
                            .xsmall()
                            .ghost()
                            .icon(IconName::PanelRightClose)
                            .tooltip("Hide info (I)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_info_open(false);
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            .child(
                resize_handle::<ResizePreview, ResizePreview>("preview-edge", Axis::Horizontal)
                    .inside(HandleEdge::Leading)
                    .on_drag(ResizePreview, |drag, _, _, cx| cx.new(|_| drag.clone())),
            )
    }

    /// Opens or closes the info pane. Closed by the user, it stays closed
    /// when another item is picked, until opened again.
    pub(crate) fn set_info_open(&mut self, open: bool) {
        self.preview_open = open;
        self.info_closed = !open;
    }

    fn render_preview_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // The image follows the pane width at 4:3, the shape of iOS thumbnails.
        let image_h = ((self.preview_w - 24.) * 0.75).max(120.);
        let pane = v_flex()
            .id("preview-pane")
            .size_full()
            .overflow_y_scrollbar()
            .p_3()
            .gap_3();

        let Some(a) = self.focused.and_then(|i| self.assets().get(i)) else {
            return pane.child(muted("Select an item to see its details.".into(), &theme));
        };

        let veiled = self.veiled(a);
        let key = a.device_path();
        let image = match (self.thumbs.get(&key), self.thumb_for(a)) {
            (Some(Thumb::Ready(_)), Some(path)) => img(path)
                .w_full()
                .h(px(image_h))
                .object_fit(if veiled {
                    ObjectFit::Cover
                } else {
                    ObjectFit::Contain
                })
                .into_any_element(),
            _ => div().w_full().h(px(image_h)).into_any_element(),
        };

        // Hidden album: blurred until the user asks to see it.
        let image = div()
            .relative()
            .rounded(px(crate::palette::RADIUS))
            .overflow_hidden()
            .bg(theme.title_bar)
            .child(image)
            .when(a.hidden, |d| {
                let key = key.clone();
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!veiled, |d| d.items_end().justify_end().p_2())
                        .child(
                            btn("preview-reveal")
                                .small()
                                .icon(if veiled {
                                    IconName::Eye
                                } else {
                                    IconName::EyeOff
                                })
                                .when(veiled, |b| b.label("Show"))
                                .tooltip(if veiled {
                                    "Hidden album: show this item"
                                } else {
                                    "Blur again"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_revealed(key.clone(), veiled);
                                    cx.notify();
                                })),
                        ),
                )
            });

        // "Video · 0:12 · 3 Mar 2026"
        let mut subtitle = vec![kind_label(a)];
        if let Some(d) = a.captured_local() {
            subtitle.push(d.format("%-d %b %Y").to_string());
        }

        let copied = self.in_dest.get(&a.device_path()).cloned().flatten();
        let actions = h_flex()
            .gap_2()
            .flex_wrap()
            .when(!a.trashed && self.dest_root().is_some(), |d| {
                d.child(
                    btn("copy-focused")
                        .small()
                        .primary()
                        .icon(IconName::Copy)
                        .label("Copy")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.copy_focused();
                            cx.notify();
                        })),
                )
            })
            .child(
                btn("open-focused")
                    .small()
                    .outline()
                    .icon(if a.kind == MediaKind::Video {
                        IconName::Video
                    } else {
                        IconName::Image
                    })
                    .label("Open")
                    .tooltip("Copy to a temporary folder and open with the system app")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_focused();
                        cx.notify();
                    })),
            )
            .when_some(copied, |d, path| {
                d.child(
                    btn("show-copy")
                        .small()
                        .ghost()
                        .icon(IconName::FolderOpen)
                        .label("Show in folder")
                        .tooltip("Show the copy in the file manager")
                        .on_click(move |_, _, _| crate::desktop::show_in_folder(&path)),
                )
            });

        pane.child(image)
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(a.filename.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(subtitle.join(" · ")),
                    ),
            )
            .child(actions)
            .when_some(self.status.clone(), |d, s| d.child(muted(s, &theme)))
            .child(info_rows(a, self.copied_to(a), &theme, cx))
    }
}

impl MainView {
    /// Where the item's copy is, relative to the Save-to folder, if it's there.
    fn copied_to(&self, a: &Asset) -> Option<String> {
        let settings = self.copy_settings()?;
        let dest = self.in_dest.get(&a.device_path())?.clone()?;
        Some(
            dest.strip_prefix(&settings.dest)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| dest.display().to_string()),
        )
    }
}

/// Friendly name for the app that saved an item.
fn source_label(a: &Asset) -> Option<String> {
    let app = a.source_app.as_deref()?;
    Some(match app {
        "Camera" => match &a.camera {
            Some(cam) => format!("Camera ({cam})"),
            None => "Camera".to_owned(),
        },
        // SpringBoard: an image saved from another app or the share sheet.
        "SpringBoard" | "Photos" => "Saved from iOS".to_owned(),
        other => other.to_owned(),
    })
}

/// "Video · 0:12", "Live Photo", "Photo".
fn kind_label(a: &Asset) -> String {
    match (a.kind, a.duration_secs) {
        (MediaKind::Video, Some(s)) => format!("Video · {}:{:02}", s as u64 / 60, s as u64 % 60),
        (MediaKind::Video, None) => "Video".to_owned(),
        (MediaKind::Photo, _) if a.live_photo => "Live Photo".to_owned(),
        (MediaKind::Photo, _) => "Photo".to_owned(),
    }
}

/// The item's details in sections: Details, Source, Location and Copy.
pub(crate) fn info_rows(
    a: &Asset,
    copied_to: Option<String>,
    theme: &Theme,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    let mut details: Vec<(&str, String)> = Vec::new();
    let kind = match (a.kind, a.duration_secs) {
        (MediaKind::Photo, _) if a.live_photo => "Live Photo (saved as a still)".to_owned(),
        _ => kind_label(a),
    };
    details.push(("Type", kind));
    if let Some(d) = a.captured_local() {
        let zone = if a.tz_offset_secs.is_some() {
            d.format(" (UTC%:z)").to_string()
        } else {
            String::new()
        };
        details.push(("Taken", format!("{}{zone}", d.format("%-d %b %Y %H:%M"))));
    }
    if let (Some(w), Some(h)) = (a.width, a.height) {
        details.push(("Resolution", format!("{w} × {h}")));
    }
    if let Some(bytes) = a.original_size {
        details.push(("File size", human_size(bytes)));
    }
    let mut flags = Vec::new();
    if a.is_edited() {
        flags.push("Edited");
    }
    if a.favorite {
        flags.push("Favourite");
    }
    if a.hidden {
        flags.push("Hidden");
    }
    if !flags.is_empty() {
        details.push(("Marked", flags.join(" · ")));
    }

    let mut source: Vec<(&str, String)> = Vec::new();
    if let Some(src) = source_label(a) {
        source.push(("App", src));
    }
    // Camera EXIF also on items saved by other apps (e.g. a Nikon photo from Drive).
    if let Some(cam) = &a.camera
        && a.source_app.as_deref() != Some("Camera")
    {
        source.push(("Camera", cam.clone()));
    }
    if let Some(lens) = &a.lens {
        source.push(("Lens", lens.clone()));
    }
    if let Some(exp) = &a.exposure {
        source.push(("Exposure", exp.clone()));
    }
    if !a.albums.is_empty() {
        source.push(("Albums", a.albums.join(", ")));
    }

    let mut location: Vec<(&str, String)> = Vec::new();
    if let Some(place) = &a.place {
        location.push(("Place", place.clone()));
    }
    if let Some((lat, lon)) = a.location {
        location.push(("Location", format!("{lat:.5}, {lon:.5}")));
    }

    let mut copy: Vec<(&str, String)> = Vec::new();
    let saves = if a.trashed {
        "Not copied (in Recently Deleted)".to_owned()
    } else {
        let plan = a.save_plan();
        let mut s = if plan.converts() {
            format!("{} → {}", plan.from_ext, plan.to_ext)
        } else {
            format!("{} (unchanged)", plan.to_ext)
        };
        if plan.uses_edited {
            s.push_str(" · edited version");
        }
        s
    };
    copy.push(("Saves as", saves));
    if !a.trashed {
        let copied = copied_to.is_some();
        copy.push((
            "Status",
            if copied { "Copied" } else { "Not copied yet" }.to_owned(),
        ));
        if let Some(rel) = copied_to {
            copy.push(("Copied to", rel));
        }
    }

    let map_link = a.location.map(|(lat, lon)| google_maps_url(lat, lon));
    let copied_green = crate::palette::hsla(crate::palette::COPIED);
    let row = |k: &'static str, v: String| {
        // Place and coordinates can be copied (gpui text isn't selectable).
        let copyable = matches!(k, "Place" | "Location");
        let text = v.clone();
        let is_copied = k == "Status" && v == "Copied";
        h_flex()
            .gap_2()
            .items_start()
            .child(
                div()
                    .w(px(76.))
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground)
                    .child(k),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(is_copied, |d| d.text_color(copied_green))
                    .child(v),
            )
            .when_some(map_link.clone().filter(|_| k == "Location"), |d, url| {
                d.child(
                    btn("open-map")
                        .xsmall()
                        .ghost()
                        .icon(IconName::MapPin)
                        .tooltip("Open in Google Maps (in your browser)")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                )
            })
            .when(copyable, |d| {
                d.child(
                    btn(SharedString::from(format!("copy-{k}")))
                        .xsmall()
                        .ghost()
                        .icon(IconName::Copy)
                        .tooltip(format!("Copy {}", k.to_lowercase()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                            this.status = Some("Copied to the clipboard".into());
                            cx.notify();
                        })),
                )
            })
    };
    let mut sections = Vec::new();
    for (title, rows) in [
        ("DETAILS", details),
        ("SOURCE", source),
        ("LOCATION", location),
        ("COPY", copy),
    ] {
        if rows.is_empty() {
            continue;
        }
        let body: Vec<_> = rows.into_iter().map(|(k, v)| row(k, v)).collect();
        sections.push(
            v_flex()
                .gap_1()
                .pt_2()
                .border_t_1()
                .border_color(theme.border)
                .child(section_label(title, theme))
                .children(body),
        );
    }
    v_flex().gap_3().text_sm().children(sections)
}

/// A Google Maps link that drops a pin at these coordinates. Opened in the
/// browser only when the user clicks; Rollcopy itself sends nothing.
pub(crate) fn google_maps_url(lat: f64, lon: f64) -> String {
    format!("https://www.google.com/maps/search/?api=1&query={lat:.6},{lon:.6}")
}

fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", (b / 1e3).max(1.))
    }
}

#[cfg(test)]
mod tests {
    use super::{google_maps_url, human_size};

    #[test]
    fn maps_link() {
        assert_eq!(
            google_maps_url(51.500_7, -0.124_6),
            "https://www.google.com/maps/search/?api=1&query=51.500700,-0.124600"
        );
        assert_eq!(
            google_maps_url(-33.86, 151.21),
            "https://www.google.com/maps/search/?api=1&query=-33.860000,151.210000"
        );
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(23_600_000), "23.6 MB");
        assert_eq!(human_size(661_115), "661 KB");
        assert_eq!(human_size(12), "1 KB");
        assert_eq!(human_size(2_500_000_000), "2.50 GB");
    }
}
