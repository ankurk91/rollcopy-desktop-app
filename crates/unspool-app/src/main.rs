//! Unspool desktop app (binary: `unspool`).

mod grid;
mod preview;
mod theme;
mod transfers;
mod tree;
mod view;
mod worker;

use gpui_kit::*;

const APP_NAME: &str = "Unspool";
const APP_ID: &str = "dev.unspool.Unspool";

// Lucide icons used by the app, on top of the component set.
gpui_kit_assets::icon_assets!(
    AppAssets,
    [
        ArrowDownWideNarrow,
        ArrowUpNarrowWide,
        Battery,
        BatteryCharging,
        ChevronDown,
        ChevronRight,
        Copy,
        FolderCheck,
        FolderOpen,
        Moon,
        Sun,
        SunMoon,
        X,
        Image,
        Images,
        Lock,
        Plug,
        RefreshCw,
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

fn app_paths() -> worker::CachePaths {
    match directories::ProjectDirs::from("dev", "unspool", "unspool") {
        Some(d) => worker::CachePaths {
            root: d.cache_dir().to_path_buf(),
            state: d
                .state_dir()
                .unwrap_or_else(|| d.data_local_dir())
                .to_path_buf(),
        },
        None => {
            let tmp = std::env::temp_dir().join("unspool");
            worker::CachePaths {
                root: tmp.clone(),
                state: tmp,
            }
        }
    }
}

fn main() {
    let paths = app_paths();
    let worker = worker::spawn(paths.clone());

    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            view::bind_keys(cx);
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
                theme::apply(theme::ThemeChoice::System, window, cx);
                cx.new(|cx| view::MainView::new(worker, paths, window, cx))
            })
            .expect("failed to open window");
        });
}
