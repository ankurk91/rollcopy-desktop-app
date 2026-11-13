//! Tray icon (PLAN §9): a flat, one-colour outline of the app icon's film
//! roll (`packaging/icons/tray.svg`), with a status dot in the empty
//! bottom-right corner. The full-colour app icon was mush at 16–22 px and
//! the dot was hard to see on its purple background.

use resvg::tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use resvg::usvg;

/// Flat tray glyph; its `#ffffff` is replaced with the panel colour.
const TRAY_SVG: &str = include_str!("../../../packaging/icons/tray.svg");

/// Glyph colour for a dark panel (most desktops) and for a light one.
pub const ON_DARK: [u8; 3] = [236, 236, 238];
pub const ON_LIGHT: [u8; 3] = [40, 40, 46];

/// Sizes offered to the tray host; it picks the closest.
const TRAY_SIZES: [u32; 3] = [22, 32, 48];

/// One rendered size, ARGB32 in network byte order (what StatusNotifierItem wants).
#[derive(Debug, Clone)]
pub struct Argb {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Renders the tray glyph in `fg` at tray sizes, with a status dot of
/// `dot` (RGB).
pub fn tray_icons(fg: [u8; 3], dot: Option<[u8; 3]>) -> Vec<Argb> {
    let colour = format!("#{:02x}{:02x}{:02x}", fg[0], fg[1], fg[2]);
    let svg = TRAY_SVG.replace("#ffffff", &colour);
    let Ok(tree) = usvg::Tree::from_str(&svg, &usvg::Options::default()) else {
        return Vec::new();
    };
    TRAY_SIZES
        .iter()
        .filter_map(|&size| render(&tree, size, dot))
        .collect()
}

fn render(tree: &usvg::Tree, size: u32, dot: Option<[u8; 3]>) -> Option<Argb> {
    let mut pm = Pixmap::new(size, size)?;
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, Transform::from_scale(scale, scale), &mut pm.as_mut());

    if let Some([r, g, b]) = dot {
        let s = size as f32;
        // Large enough to read at 16 px; the corner it sits in is empty.
        let radius = s * 0.24;
        let (cx, cy) = (s - radius - 0.3, s - radius - 0.3);
        circle(&mut pm, cx, cy, radius, [r, g, b]);
    }

    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        data.extend([c.alpha(), c.red(), c.green(), c.blue()]);
    }
    Some(Argb {
        width: size,
        height: size,
        data,
    })
}

fn circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32, [red, green, blue]: [u8; 3]) {
    let Some(path) = PathBuilder::from_circle(cx, cy, r) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(red, green, blue, 255);
    paint.anti_alias = true;
    pm.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// Reads one pixel (ARGB) of a rendered icon, for tests.
#[cfg(test)]
fn pixel(icon: &Argb, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * icon.width + x) * 4) as usize;
    [
        icon.data[i],
        icon.data[i + 1],
        icon.data[i + 2],
        icon.data[i + 3],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_sizes() {
        let icons = tray_icons(ON_DARK, None);
        assert_eq!(icons.len(), 3);
        for i in &icons {
            assert_eq!(i.data.len(), (i.width * i.height * 4) as usize);
        }
        // A line of the roll is drawn in the glyph colour; the background
        // is transparent.
        let icon = &icons[2];
        let drawn = (0..icon.width)
            .flat_map(|x| (0..icon.height).map(move |y| (x, y)))
            .map(|(x, y)| pixel(icon, x, y))
            .filter(|p| p[0] == 255)
            .count();
        assert!(drawn > 50, "{drawn}");
        assert_eq!(pixel(icon, 0, icon.height - 1)[0], 0, "corner transparent");
        let any = (0..icon.width * icon.height)
            .map(|i| pixel(icon, i % icon.width, i / icon.width))
            .find(|p| p[0] == 255)
            .unwrap();
        assert_eq!(&any[1..], &ON_DARK, "glyph colour");
    }

    #[test]
    fn dot_is_drawn_bottom_right() {
        let icon = &tray_icons(ON_LIGHT, Some([239, 68, 68]))[2];
        let s = icon.width;
        let r = s as f32 * 0.24;
        let c = (s as f32 - r - 0.3) as u32;
        let [a, red, green, blue] = pixel(icon, c, c);
        assert_eq!(a, 255);
        assert!(
            red > 200 && green < 100 && blue < 100,
            "{red},{green},{blue}"
        );
    }
}
