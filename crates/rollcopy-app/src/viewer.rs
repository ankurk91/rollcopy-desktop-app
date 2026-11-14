//! Full-window viewer (PLAN §6: "Double-click opens fullscreen with ← / →").
//!
//! Shows the focused item over the whole window. Photos are fetched at full
//! size by the worker (HEIC converted to JPG); the thumbnail shows while that
//! loads. Videos show their poster frame with a Play button that hands the
//! file to the system player. Arrow keys reuse the grid's cursor movement, so
//! the grid selection follows the viewer.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Icon, Selectable as _, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use rollcopy_library::MediaKind;

use crate::view::{MainView, Thumb, btn};
use crate::worker::Command;

const HEADER_H: f32 = 44.;
const FOOTER_H: f32 = 28.;
/// Room on each side for the ← → buttons.
const SIDE: f32 = 64.;
const BOTTOM: f32 = 8.;

/// Size to draw an image of `img` pixels so it fits inside `room`, keeping
/// its aspect ratio (scaled up or down). gpui's `img()` gives itself the
/// image's aspect ratio and, inside a flexible box, grew to the full width
/// and spilled out at the bottom, so the viewer sizes images itself.
pub(crate) fn fit(img: (u32, u32), room: (f32, f32)) -> (f32, f32) {
    let (iw, ih) = (img.0.max(1) as f32, img.1.max(1) as f32);
    let (rw, rh) = (room.0.max(1.), room.1.max(1.));
    let scale = (rw / iw).min(rh / ih);
    (iw * scale, ih * scale)
}

/// State of a full-size image, by asset device path.
pub(crate) enum Full {
    Pending,
    Ready(PathBuf),
    Failed(String),
}

impl MainView {
    /// Space / Enter / double-click: photos open in the viewer, videos in
    /// the system player.
    pub(crate) fn activate_focused(&mut self) {
        let Some(idx) = self.focused else {
            return;
        };
        if self.assets().get(idx).map(|a| a.kind) == Some(MediaKind::Photo) {
            self.viewer_open = true;
        } else {
            self.open_focused();
        }
    }

    /// Asks the worker for a full-size copy of a photo, once.
    fn request_full(&mut self, idx: usize) {
        let Some(udid) = self.udid().map(str::to_owned) else {
            return;
        };
        let Some(a) = self.assets().get(idx) else {
            return;
        };
        // Hidden album: nothing full-size until the user asks to see it.
        if a.kind != MediaKind::Photo || self.veiled(a) {
            return;
        }
        let key = a.device_path();
        if self.full.contains_key(&key) {
            return;
        }
        let dest = self.paths.view(&udid, a);
        if dest.exists() {
            self.full.insert(key, Full::Ready(dest));
            return;
        }
        let source = a.save_plan().source;
        self.full.insert(key.clone(), Full::Pending);
        let _ = self.commands.try_send(Command::FullImage {
            udid,
            key,
            source,
            dest,
        });
    }

    /// An image file's pixel size, cached.
    fn dims_of(&mut self, path: &std::path::Path) -> Option<(u32, u32)> {
        *self
            .image_dims
            .entry(path.to_path_buf())
            .or_insert_with(|| rollcopy_convert::image_size(path))
    }

    /// Zooms the viewer's photo by `factor` (fit = 1).
    pub(crate) fn zoom_viewer(&mut self, factor: f32, cx: &mut Context<Self>) {
        let next = (self.viewer_zoom * factor).clamp(1., self.viewer_max_zoom);
        if next != self.viewer_zoom {
            if next == 1. {
                self.viewer_pan = (0., 0.);
            }
            self.viewer_zoom = next;
            cx.notify();
        }
    }

    pub(crate) fn reset_viewer_zoom(&mut self) {
        self.viewer_zoom = 1.;
        self.viewer_pan = (0., 0.);
        self.viewer_drag = None;
    }

    pub(crate) fn render_viewer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.viewer_open {
            return None;
        }
        let idx = self.focused?;
        let pos = self.visible.iter().position(|&v| v == idx)?;
        // This item and its neighbours, so ← → feel instant.
        self.request_full(idx);
        if let Some(&prev) = pos.checked_sub(1).and_then(|p| self.visible.get(p)) {
            self.request_full(prev);
        }
        if let Some(&next) = self.visible.get(pos + 1) {
            self.request_full(next);
        }

        // Zoom and pan belong to one photo.
        if self.viewer_for != Some(idx) {
            self.viewer_for = Some(idx);
            self.reset_viewer_zoom();
        }
        let a = self.assets().get(idx)?.clone();
        let key = a.device_path();
        let total = self.visible.len();
        let veiled = self.veiled(&a);
        let thumb = match self.thumbs.get(&key) {
            Some(Thumb::Ready(_)) => self.thumb_for(&a),
            _ => None,
        };
        // The box the image is drawn in: the full image's own size once it's
        // here, else the item's size from the library, else the thumbnail's.
        let full_path = match self.full.get(&key) {
            Some(Full::Ready(p)) if a.kind == MediaKind::Photo => Some(p.clone()),
            _ => None,
        };
        let dims = full_path
            .as_ref()
            .and_then(|p| self.dims_of(p))
            .or_else(|| a.width.zip(a.height))
            .or_else(|| thumb.as_ref().and_then(|p| self.dims_of(p)))
            .unwrap_or((4, 3));
        let viewport = window.viewport_size();
        let room = (
            f32::from(viewport.width) - 2. * SIDE,
            f32::from(viewport.height) - HEADER_H - FOOTER_H - BOTTOM,
        );
        let (fit_w, fit_h) = fit(dims, room);
        // Zoom works on the full photo only (not a thumbnail or a video).
        let zoomable = full_path.is_some() && !veiled;
        self.viewer_max_zoom = if zoomable {
            (dims.0 as f32 / fit_w).clamp(2., 8.)
        } else {
            1.
        };
        if !zoomable {
            self.reset_viewer_zoom();
        }
        let zoom = self.viewer_zoom;
        let (w, h) = (fit_w * zoom, fit_h * zoom);
        // Keep the photo covering its box: pan at most to its edges.
        let max_pan = (((w - room.0) / 2.).max(0.), ((h - room.1) / 2.).max(0.));
        self.viewer_pan = (
            self.viewer_pan.0.clamp(-max_pan.0, max_pan.0),
            self.viewer_pan.1.clamp(-max_pan.1, max_pan.1),
        );
        let pan = self.viewer_pan;
        let copied_path = self.in_dest.get(&key).cloned().flatten();
        let copied_rel = self.copy_settings().and_then(|s| {
            copied_path.as_ref().map(|p| {
                p.strip_prefix(&s.dest)
                    .map(|r| r.display().to_string())
                    .unwrap_or_else(|_| p.display().to_string())
            })
        });
        let contained = move |p: PathBuf| {
            img(p)
                .w(px(w))
                .h(px(h))
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        };

        let reveal_key = key.clone();
        let (image, overlay): (AnyElement, Option<AnyElement>) = match a.kind {
            _ if veiled => (
                thumb
                    .map(contained)
                    .unwrap_or_else(|| div().into_any_element()),
                Some(
                    btn("viewer-reveal")
                        .large()
                        .icon(IconName::Eye)
                        .label("Show hidden item")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_revealed(reveal_key.clone(), true);
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            ),
            MediaKind::Photo => match self.full.get(&key) {
                Some(Full::Ready(p)) => (contained(p.clone()), None),
                Some(Full::Failed(e)) => (
                    thumb
                        .map(contained)
                        .unwrap_or_else(|| div().into_any_element()),
                    Some(
                        div()
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .bg(hsla(0., 0., 0., 0.7))
                            .text_sm()
                            .child(format!("Couldn't load the full photo: {e}"))
                            .into_any_element(),
                    ),
                ),
                _ => (
                    thumb
                        .map(contained)
                        .unwrap_or_else(|| div().into_any_element()),
                    Some(Spinner::new().large().into_any_element()),
                ),
            },
            MediaKind::Video => (
                thumb
                    .map(contained)
                    .unwrap_or_else(|| div().into_any_element()),
                Some(
                    btn("viewer-play")
                        .large()
                        .primary()
                        .icon(IconName::Video)
                        .label("Play")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_focused();
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            ),
        };

        let when = a
            .captured_local()
            .map(|d| d.format(" · %-d %b %Y %H:%M").to_string())
            .unwrap_or_default();
        let nav = |id: &'static str, icon: IconName, delta: isize, enabled: bool| {
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .px_3()
                .when(delta < 0, |d| d.left_0())
                .when(delta > 0, |d| d.right_0())
                .when(enabled, |d| {
                    d.child(btn(id).large().ghost().icon(icon).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.move_cursor(delta, false);
                            cx.notify();
                        },
                    )))
                })
        };
        let left = nav("viewer-prev", IconName::ChevronLeft, -1, pos > 0);
        let right = nav("viewer-next", IconName::ChevronRight, 1, pos + 1 < total);

        Some(
            v_flex()
                .id("viewer")
                .absolute()
                .inset_0()
                .bg(hsla(0., 0., 0.04, 1.))
                .text_color(white())
                // Clicks stay in the viewer.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    h_flex()
                        .w_full()
                        .h(px(HEADER_H))
                        .flex_shrink_0()
                        .px_4()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(a.filename.clone()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(hsla(0., 0., 0.75, 1.))
                                .child(format!(
                                    "{}{when}",
                                    if a.is_edited() { "Edited" } else { "" }
                                )),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_sm()
                                .text_color(hsla(0., 0., 0.75, 1.))
                                .child(format!("{} / {total}", pos + 1)),
                        )
                        .when_some(copied_path.clone(), |d, path| {
                            d.child(
                                btn("viewer-show")
                                    .small()
                                    .ghost()
                                    .icon(IconName::FolderOpen)
                                    .label("Show in folder")
                                    .tooltip("Open the copy in the file manager")
                                    .on_click(move |_, _, _| crate::desktop::show_in_folder(&path)),
                            )
                        })
                        .child(
                            btn("viewer-info-toggle")
                                .small()
                                .ghost()
                                .icon(IconName::Info)
                                .tooltip("Details (I)")
                                .when(self.viewer_info, |b| b.selected(true))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.viewer_info = !this.viewer_info;
                                    cx.notify();
                                })),
                        )
                        .child(
                            btn("viewer-close")
                                .small()
                                .ghost()
                                .icon(IconName::X)
                                .tooltip("Close (Esc)")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.viewer_open = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("viewer-photo")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .pb(px(BOTTOM))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(zoomable && zoom > 1., |d| {
                            d.cursor(if self.viewer_drag.is_some() {
                                CursorStyle::ClosedHand
                            } else {
                                CursorStyle::OpenHand
                            })
                        })
                        // Wheel zooms; double-click toggles fit / 100%.
                        .on_scroll(cx.listener(move |this, ev: &ScrollEvent, _, cx| {
                            if !zoomable {
                                return;
                            }
                            let dy = match ev.delta {
                                ScrollDelta::Lines(p) => p.y,
                                ScrollDelta::Pixels(p) => p.y / px(1.) / 40.,
                            };
                            if dy != 0. {
                                this.zoom_viewer(1.15_f32.powf(dy.clamp(-3., 3.)), cx);
                            }
                        }))
                        .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                            if zoomable && ev.click_count() >= 2 {
                                if this.viewer_zoom > 1. {
                                    this.reset_viewer_zoom();
                                } else {
                                    let actual = this.viewer_max_zoom.min(dims.0 as f32 / fit_w);
                                    this.viewer_zoom = actual.max(2.).min(this.viewer_max_zoom);
                                }
                                cx.notify();
                            }
                        }))
                        // Drag to move a zoomed photo.
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, _, _| {
                                if this.viewer_zoom > 1. {
                                    this.viewer_drag = Some(ev.position);
                                }
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                            let Some(last) = this.viewer_drag else {
                                return;
                            };
                            if ev.pressed_button != Some(MouseButton::Left) {
                                this.viewer_drag = None;
                                return;
                            }
                            this.viewer_pan.0 += (ev.position.x - last.x) / px(1.);
                            this.viewer_pan.1 += (ev.position.y - last.y) / px(1.);
                            this.viewer_drag = Some(ev.position);
                            cx.notify();
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                this.viewer_drag = None;
                                cx.notify();
                            }),
                        )
                        .child(
                            div()
                                .relative()
                                .left(px(pan.0))
                                .top(px(pan.1))
                                .child(image),
                        )
                        .when_some(overlay, |d, o| {
                            d.child(
                                div()
                                    .absolute()
                                    .inset_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(o),
                            )
                        })
                        .child(left)
                        .child(right)
                        .when(self.viewer_info, |d| {
                            let theme = cx.theme().clone();
                            d.child(
                                div()
                                    .id("viewer-info")
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .w(px(320.))
                                    .p_4()
                                    .bg(theme.background)
                                    .text_color(theme.foreground)
                                    .border_l_1()
                                    .border_color(theme.border)
                                    .overflow_y_scroll()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation()
                                    })
                                    .child(crate::preview::info_rows(
                                        &a,
                                        copied_rel.clone(),
                                        &theme,
                                        cx,
                                    )),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .h(px(FOOTER_H))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(hsla(0., 0., 0.6, 1.))
                                .child(if zoomable {
                                    "← → move   scroll / double-click zoom   drag move   I details   Esc close"
                                } else {
                                    "← → move   I details   Esc close"
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// Small helper so the empty-state code can show a large icon.
pub(crate) fn big_icon(icon: IconName, color: Hsla) -> impl IntoElement {
    Icon::new(icon).with_size(px(48.)).text_color(color)
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn fits_inside_the_room_keeping_shape() {
        // Tall photo in a wide window: limited by height.
        let (w, h) = fit((899, 1599), (1028., 700.));
        assert_eq!(h, 700.);
        assert!((w - 393.56).abs() < 0.01, "{w}");
        // Wide photo: limited by width.
        assert_eq!(fit((4000, 3000), (1000., 1000.)), (1000., 750.));
        // Small images are scaled up to fill.
        assert_eq!(fit((480, 360), (960., 960.)), (960., 720.));
        // Degenerate input doesn't divide by zero.
        let (w, h) = fit((0, 0), (0., 0.));
        assert!(w.is_finite() && h.is_finite());
    }
}
