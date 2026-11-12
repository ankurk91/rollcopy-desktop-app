//! Rollcopy desktop app (binary: `rollcopy`).

mod about;
mod config;
mod grid;
mod icons;
mod notify;
mod prefs;
mod prefs_window;
mod preview;
mod theme;
mod transfers;
mod tray;
mod tree;
mod update;
mod view;
mod viewer;
mod worker;

use std::borrow::Cow;

use gpui_kit::*;

const APP_NAME: &str = "Rollcopy";
const APP_ID: &str = "io.github.ankurk91.Rollcopy";
/// Smallest main window: room for the side pane and one grid column.
const MIN_W: f32 = 640.;
const MIN_H: f32 = 480.;

// Lucide icons used by the app, on top of the component set.
gpui_kit_assets::icon_assets!(
    AppAssets,
    [
        ArrowDownWideNarrow,
        ArrowUpNarrowWide,
        Battery,
        BatteryCharging,
        ChevronDown,
        ChevronLeft,
        ChevronRight,
        Copy,
        FolderCheck,
        FolderOpen,
        Menu,
        Moon,
        Sun,
        SunMoon,
        Trash,
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

/// Lucide icons plus the full-colour app icon (for the About dialog).
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == about::APP_ICON {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../../../packaging/icons/io.github.ankurk91.Rollcopy.svg"
            ))));
        }
        AppAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        AppAssets.list(path)
    }
}

/// Saved window geometry, if it still makes sense.
fn restored_bounds(cx: &App) -> Option<WindowBounds> {
    let w = prefs::Prefs::get(cx).ui.window?;
    if w.width < MIN_W || w.height < MIN_H {
        return None;
    }
    let b = Bounds {
        origin: point(px(w.x), px(w.y)),
        size: size(px(w.width), px(w.height)),
    };
    Some(if w.maximized {
        WindowBounds::Maximized(b)
    } else {
        WindowBounds::Windowed(b)
    })
}

fn app_paths() -> worker::CachePaths {
    match directories::ProjectDirs::from("io.github", "ankurk91", "rollcopy") {
        Some(d) => worker::CachePaths {
            root: d.cache_dir().to_path_buf(),
            state: d
                .state_dir()
                .unwrap_or_else(|| d.data_local_dir())
                .to_path_buf(),
        },
        None => {
            let tmp = std::env::temp_dir().join("rollcopy");
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

    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(prefs::Prefs::load());
        view::bind_keys(cx);
        // App-wide actions; the main window handles Quit itself first so
        // it can save its state.
        cx.on_action(|_: &view::OpenPreferences, cx| prefs_window::open(cx));
        cx.on_action(|_: &view::ViewOnGitHub, cx| cx.open_url(update::REPO_URL));
        cx.on_action(|_: &view::ReportIssue, cx| cx.open_url(update::ISSUES_URL));
        cx.on_action(|_: &view::Quit, cx| cx.quit());

        let options = WindowOptions {
            window_bounds: Some(
                restored_bounds(cx)
                    .unwrap_or_else(|| WindowBounds::centered(size(px(1100.), px(720.)), cx)),
            ),
            app_id: Some(APP_ID.into()),
            window_min_size: Some(size(px(MIN_W), px(MIN_H))),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (main_window, _) = gpui_kit::open_window(options, cx, move |window, cx| {
            let choice = prefs::Prefs::get(cx).config.app.theme.into();
            theme::apply(choice, window, cx);
            cx.new(|cx| view::MainView::new(worker, paths, window, cx))
        })
        .expect("failed to open window");
        // Closing the main window quits; closing Preferences doesn't.
        let main_id = main_window.window_id();
        cx.on_window_closed(move |cx, id| {
            if id == main_id {
                cx.quit();
            }
        })
        .detach();
    });
}
