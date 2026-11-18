//! Light/dark handling (PLAN §6, §8a "Theme: System / Light / Dark").
//!
//! Both modes are painted from `palette.rs`, over gpui-component's stock
//! theme. Colours are re-applied after every theme change because
//! `Theme::change` reloads the stock palette.

use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::palette::{self, Palette};

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
    let p = if mode == ThemeMode::Dark {
        palette::DARK
    } else {
        palette::LIGHT
    };
    Theme::update(cx, |t| paint(t, &p));
    // Linux defaults to auto-hiding scrollbars; keep them visible so long
    // grids and lists show where you are.
    Theme::set_scrollbar_mode(ScrollbarMode::Always, cx);
}

/// Sets every theme token Rollcopy and its components draw with.
fn paint(t: &mut Theme, p: &Palette) {
    let c = palette::hsla;
    let accent = c(p.accent);
    let clear = Hsla::transparent_black();

    t.radius = px(palette::RADIUS);
    t.radius_lg = px(palette::RADIUS_LG);

    // Surfaces and text.
    t.background = c(p.content);
    t.foreground = c(p.text);
    t.muted = c(p.elevated);
    t.muted_foreground = c(p.text_muted);
    t.border = c(p.separator);
    t.input = c(p.separator);
    t.window_border = c(p.separator);
    t.popover = c(p.elevated);
    t.popover_foreground = c(p.text);
    t.group_box = c(p.elevated);
    t.group_box_foreground = c(p.text);
    t.title_bar = c(p.toolbar);
    t.title_bar_border = c(p.separator);
    t.status_bar = c(p.window);
    t.status_bar_border = c(p.separator);

    // Selection and hover: menus and lists highlight in the accent.
    t.accent = c(p.selection);
    t.accent_foreground = c(p.text);
    t.selection = accent.opacity(0.35);
    t.ring = accent;
    t.caret = c(p.text);
    t.link = c(p.accent_text);
    t.link_hover = c(p.accent_hover);
    t.link_active = c(p.accent_active);

    // Buttons. Plain buttons sit on the elevated grey; primary ones are
    // the accent.
    t.primary = accent;
    t.primary_hover = c(p.accent_hover);
    t.primary_active = c(p.accent_active);
    t.primary_foreground = c(p.on_accent);
    t.button_primary = accent;
    t.button_primary_hover = c(p.accent_hover);
    t.button_primary_active = c(p.accent_active);
    t.button_primary_foreground = c(p.on_accent);
    t.secondary = c(p.elevated);
    t.secondary_hover = c(p.separator);
    t.secondary_active = c(p.separator);
    t.secondary_foreground = c(p.text);
    t.button = c(p.elevated);
    t.button_hover = c(p.separator);
    t.button_active = c(p.separator);
    t.button_foreground = c(p.text);

    // Sidebar.
    t.sidebar = c(p.sidebar);
    t.sidebar_foreground = c(p.text);
    t.sidebar_border = c(p.separator);
    t.sidebar_accent = c(p.selection);
    t.sidebar_accent_foreground = c(p.text);
    t.sidebar_primary = accent;
    t.sidebar_primary_foreground = c(p.on_accent);

    // Lists and tables.
    t.colors.list = c(p.content);
    t.list_even = c(p.content);
    t.list_head = c(p.toolbar);
    t.list_hover = c(p.elevated);
    t.list_active = c(p.selection);
    t.list_active_border = accent;
    t.table = c(p.content);
    t.table_even = c(p.content);
    t.table_head = c(p.toolbar);
    t.table_head_foreground = c(p.text_muted);
    t.table_hover = c(p.elevated);
    t.table_active = c(p.selection);
    t.table_active_border = accent;
    t.table_row_border = c(p.separator).opacity(0.5);

    // Tabs and segmented controls.
    t.tab_bar = c(p.toolbar);
    t.tab_bar_segmented = c(p.window);
    t.tab = clear;
    t.tab_active = c(p.elevated);
    t.tab_foreground = c(p.text_muted);
    t.tab_active_foreground = c(p.text);

    // Controls.
    t.progress_bar = accent;
    t.slider_bar = accent;
    t.slider_thumb = c(p.on_accent);
    t.switch = c(p.separator);
    t.drag_border = accent;
    t.drop_target = accent.opacity(0.15);
    t.scrollbar = clear;
    t.scrollbar_thumb = c(p.separator);
    t.scrollbar_thumb_hover = c(p.text_muted);

    // Status colours.
    t.success = c(p.success);
    t.warning = c(p.warning);
    t.danger = c(p.danger);
}
