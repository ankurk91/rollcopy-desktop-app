//! Light/dark handling (PLAN §6, §8a "Theme: System / Light / Dark").
//!
//! The stock dark theme is near-black; Rollcopy's dark mode uses softer greys.
//! Colours are re-applied after every theme change because `Theme::change`
//! reloads the registered palette.

use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Follow the desktop's light/dark setting.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn next(self) -> Self {
        match self {
            ThemeChoice::System => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "Theme: follow system",
            ThemeChoice::Light => "Theme: light",
            ThemeChoice::Dark => "Theme: dark",
        }
    }
}

/// Applies the theme for `choice` to every window.
pub fn apply(choice: ThemeChoice, window: &mut Window, cx: &mut App) {
    let mode: ThemeMode = match choice {
        ThemeChoice::System => window.appearance().into(),
        ThemeChoice::Light => ThemeMode::Light,
        ThemeChoice::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, Some(window), cx);
    if mode == ThemeMode::Dark {
        Theme::update(cx, soften_dark);
    }
    // Linux defaults to auto-hiding scrollbars; keep them visible so long
    // grids and lists show where you are.
    Theme::set_scrollbar_mode(ScrollbarMode::Always, cx);
}

/// Lighter, neutral greys for dark mode.
fn soften_dark(t: &mut Theme) {
    let c = |hex: u32| -> Hsla { rgb(hex).into() };
    t.background = c(0x2b2d31);
    t.sidebar = c(0x26282c);
    t.muted = c(0x3a3c41);
    t.secondary = c(0x383a3f);
    t.accent = c(0x44464c);
    t.border = c(0x404248);
    t.popover = c(0x313338);
    t.title_bar = c(0x26282c);
}
