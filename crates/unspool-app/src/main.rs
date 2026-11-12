//! Unspool desktop app (binary: `unspool`).

mod tree;
mod view;
mod worker;

use gpui_kit::component::Theme;
use gpui_kit::*;

const APP_NAME: &str = "Unspool";
const APP_ID: &str = "dev.unspool.Unspool";

// Lucide icons used by the app, on top of the component set.
gpui_kit_assets::icon_assets!(
    AppAssets,
    [
        Battery,
        BatteryCharging,
        ChevronDown,
        ChevronRight,
        Copy,
        Image,
        Images,
        Lock,
        Plug,
        ShieldCheck,
        ShieldX,
        Smartphone,
        Terminal,
        TriangleAlert,
        Unplug,
        Usb,
        Video,
    ]
);

fn cache_dir() -> std::path::PathBuf {
    directories::ProjectDirs::from("dev", "unspool", "unspool")
        .map(|d| d.cache_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("unspool"))
}

fn main() {
    let events = worker::spawn(cache_dir());

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.on_window_closed(|cx, _| cx.quit()).detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
                app_id: Some(APP_ID.into()),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
                cx.new(|cx| view::MainView::new(events, window, cx))
            })
            .expect("failed to open window");
        });
}
