//! App icon rasterised for the tray (PLAN §9): the packaging SVG with an
//! optional status dot in the bottom-right corner.

use resvg::tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use resvg::usvg;

/// Coloured app icon, also installed for the `.desktop` file.
pub const APP_ICON_SVG: &str =
    include_str!("../../../packaging/icons/io.github.ankurk91.Rollcopy.svg");

/// Sizes offered to the tray host; it picks the closest.
const TRAY_SIZES: [u32; 3] = [22, 32, 48];

/// One rendered size, ARGB32 in network byte order (what StatusNotifierItem wants).
#[derive(Debug, Clone)]
pub struct Argb {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Renders the app icon at tray sizes, with a status dot of `dot` (RGB).
pub fn tray_icons(dot: Option<[u8; 3]>) -> Vec<Argb> {
    let Ok(tree) = usvg::Tree::from_str(APP_ICON_SVG, &usvg::Options::default()) else {
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
        let radius = s * 0.2;
        let (cx, cy) = (s - radius - 0.5, s - radius - 0.5);
        // Dark ring first so the dot reads on the blue icon and any panel.
        circle(&mut pm, cx, cy, radius + s * 0.06, [24, 26, 30]);
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
        let icons = tray_icons(None);
        assert_eq!(icons.len(), 3);
        for i in &icons {
            assert_eq!(i.data.len(), (i.width * i.height * 4) as usize);
        }
        // The middle of the icon is the opaque blue square.
        let mid = pixel(&icons[2], 6, 24);
        assert_eq!(mid[0], 255, "opaque");
        assert!(mid[3] > mid[1], "blue: {mid:?}");
    }

    #[test]
    fn dot_is_drawn_bottom_right() {
        let icon = &tray_icons(Some([239, 68, 68]))[2];
        let s = icon.width;
        let r = s as f32 * 0.2;
        let c = (s as f32 - r - 0.5) as u32;
        let [a, red, green, blue] = pixel(icon, c, c);
        assert_eq!(a, 255);
        assert!(
            red > 200 && green < 100 && blue < 100,
            "{red},{green},{blue}"
        );
    }
}
