//! Rollcopy's colours, in one place. Change a value here and the whole app
//! follows: `theme.rs` paints gpui-component's theme tokens from these, and
//! the panes only use theme tokens (plus the few named colours below).
//!
//! The look: dark graphite surfaces separated by thin lines, one purple
//! accent taken from the app icon for selection and actions, green for
//! "copied". Light mode is the same layout in light greys.

use gpui_kit::{Hsla, rgb};

/// Every colour a theme needs, as `0xRRGGBB`.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Behind everything; the transfer dock.
    pub window: u32,
    pub sidebar: u32,
    /// The grid, lists and dialogs.
    pub content: u32,
    /// Toolbar / title bar.
    pub toolbar: u32,
    /// Raised surfaces: menus, popovers, chips, hover.
    pub elevated: u32,
    pub text: u32,
    pub text_muted: u32,
    /// Thin lines between areas.
    pub separator: u32,
    /// Buttons, focus ring, progress, the selection outline on thumbnails.
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_active: u32,
    /// Text on an accent fill.
    pub on_accent: u32,
    /// Accent for links and accent-coloured text (readable on `content`).
    pub accent_text: u32,
    /// Fill of a selected row (sidebar, list, menus).
    pub selection: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
}

/// Dark graphite.
pub const DARK: Palette = Palette {
    window: 0x1e1f23,
    sidebar: 0x24252a,
    content: 0x292a30,
    toolbar: 0x222329,
    elevated: 0x303137,
    text: 0xf1f1f3,
    text_muted: 0xa0a1a8,
    separator: 0x414249,
    accent: 0x7a4ee6,
    accent_hover: 0x8a62ec,
    accent_active: 0x6a3fd6,
    on_accent: 0xffffff,
    accent_text: 0xb39af7,
    selection: 0x4e3a82,
    success: 0x43a66b,
    warning: 0xe0a33a,
    danger: 0xe5534b,
};

/// Light grey, same structure.
pub const LIGHT: Palette = Palette {
    window: 0xe6e6ea,
    sidebar: 0xececf0,
    content: 0xf7f7f9,
    toolbar: 0xe9e9ed,
    elevated: 0xffffff,
    text: 0x1d1d21,
    text_muted: 0x6b6c73,
    separator: 0xd2d2d8,
    accent: 0x6c3bd9,
    accent_hover: 0x7a4ee6,
    accent_active: 0x5a2ec4,
    on_accent: 0xffffff,
    accent_text: 0x5a2ec4,
    selection: 0xdcd2f5,
    success: 0x2f8f57,
    warning: 0xb7791f,
    danger: 0xc93c35,
};

/// Corner radius for buttons, inputs and chips: small, desktop-like.
pub const RADIUS: f32 = 4.;
/// Corner radius for dialogs and popovers.
pub const RADIUS_LG: f32 = 6.;

/// "Copied" marks on thumbnails and list rows (on a dark badge, so the
/// same in both themes).
pub const COPIED: u32 = DARK.success;

pub fn hsla(hex: u32) -> Hsla {
    rgb(hex).into()
}
