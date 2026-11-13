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
        div()
            .relative()
            .w(px(self.preview_w))
            .h_full()
            .flex_shrink_0()
            .border_l_1()
            .border_color(theme.border)
            .child(body)
            .child(
                resize_handle::<ResizePreview, ResizePreview>("preview-edge", Axis::Horizontal)
                    .inside(HandleEdge::Leading)
                    .on_drag(ResizePreview, |drag, _, _, cx| cx.new(|_| drag.clone())),
            )
    }

    fn render_preview_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // The image follows the pane width at 4:3, the shape of iOS thumbnails.
        let image_h = ((self.preview_w - 32.) * 0.75).max(120.);
        let pane = v_flex()
            .id("preview-pane")
            .size_full()
            .overflow_y_scrollbar()
            .p_4()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .child(section_label("PREVIEW", &theme))
                    .child(div().flex_1())
                    .child(
                        btn("close-preview")
                            .xsmall()
                            .ghost()
                            .icon(IconName::X)
                            .tooltip("Close preview")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.preview_open = false;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            })),
                    ),
            );

        let Some(a) = self.focused.and_then(|i| self.assets().get(i)) else {
            return pane.child(muted("Select an item to see its details.".into(), &theme));
        };

        let image = match self.thumbs.get(&a.device_path()) {
            Some(Thumb::Ready(path)) => img(path.clone())
                .w_full()
                .h(px(image_h))
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            _ => div()
                .w_full()
                .h(px(image_h))
                .rounded_md()
                .bg(theme.muted)
                .into_any_element(),
        };

        let open_label = if a.kind == MediaKind::Video {
            "Open ▶"
        } else {
            "Open"
        };

        pane.child(image)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(a.filename.clone()),
            )
            .child(info_rows(a, self.copied_to(a), &theme, cx))
            .child(
                h_flex()
                    .gap_2()
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
                            .label(open_label)
                            .tooltip("Copy to a temporary folder and open with the system app")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_focused();
                                cx.notify();
                            })),
                    ),
            )
            .when_some(self.status.clone(), |d, s| d.child(muted(s, &theme)))
    }
}

impl MainView {
    /// Where the item's copy is, relative to the Save-to folder, if it's there.
    fn copied_to(&self, a: &Asset) -> Option<String> {
        let settings = self.copy_settings()?;
        let dest = match self.in_dest.get(&a.device_path()) {
            Some(known) => known.clone()?,
            None => rollcopy_queue::find_copy(&settings, a)?,
        };
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

fn info_rows(
    a: &Asset,
    copied_to: Option<String>,
    theme: &Theme,
    cx: &mut Context<MainView>,
) -> impl IntoElement {
    let mut rows: Vec<(&str, String)> = Vec::new();

    let kind = match (a.kind, a.duration_secs) {
        (MediaKind::Video, Some(s)) => format!("Video · {}:{:02}", s as u64 / 60, s as u64 % 60),
        (MediaKind::Video, None) => "Video".to_owned(),
        (MediaKind::Photo, _) if a.live_photo => "Live Photo (saved as a still)".to_owned(),
        (MediaKind::Photo, _) => "Photo".to_owned(),
    };
    rows.push(("Type", kind));

    if let Some(d) = a.captured_local() {
        let zone = if a.tz_offset_secs.is_some() {
            d.format(" (UTC%:z)").to_string()
        } else {
            String::new()
        };
        rows.push(("Taken", format!("{}{zone}", d.format("%-d %b %Y %H:%M"))));
    }
    if let (Some(w), Some(h)) = (a.width, a.height) {
        rows.push(("Size", format!("{w} × {h}")));
    }
    if let Some(bytes) = a.original_size {
        rows.push(("File size", human_size(bytes)));
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
        rows.push(("", flags.join(" · ")));
    }
    if let Some(src) = source_label(a) {
        rows.push(("Source", src));
    }
    // Camera EXIF also on items saved by other apps (e.g. a Nikon photo from Drive).
    if let Some(cam) = &a.camera
        && a.source_app.as_deref() != Some("Camera")
    {
        rows.push(("Camera", cam.clone()));
    }
    if let Some(lens) = &a.lens {
        rows.push(("Lens", lens.clone()));
    }
    if let Some(exp) = &a.exposure {
        rows.push(("Exposure", exp.clone()));
    }
    if let Some(place) = &a.place {
        rows.push(("Place", place.clone()));
    }
    if let Some((lat, lon)) = a.location {
        rows.push(("Location", format!("{lat:.5}, {lon:.5}")));
    }
    if !a.albums.is_empty() {
        rows.push(("Albums", a.albums.join(", ")));
    }

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
    rows.push(("Saves as", saves));
    if !a.trashed {
        rows.push((
            "Copied",
            match copied_to {
                Some(rel) => format!("Yes · {rel}"),
                None => "Not yet".to_owned(),
            },
        ));
    }

    v_flex()
        .gap_1()
        .text_sm()
        .children(rows.into_iter().map(|(k, v)| {
            // Place and coordinates can be copied (gpui text isn't selectable).
            let copyable = matches!(k, "Place" | "Location");
            let text = v.clone();
            h_flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .w(px(72.))
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(k.to_owned()),
                )
                .child(div().flex_1().min_w_0().child(v))
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
        }))
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
    use super::human_size;

    #[test]
    fn sizes() {
        assert_eq!(human_size(23_600_000), "23.6 MB");
        assert_eq!(human_size(661_115), "661 KB");
        assert_eq!(human_size(12), "1 KB");
        assert_eq!(human_size(2_500_000_000), "2.50 GB");
    }
}
