//! Rollcopy desktop app (binary: `rollcopy`).

mod about;
mod config;
mod grid;
mod icons;
mod instance;
mod log;
mod notify;
mod paths;
mod prefs;
mod prefs_window;
mod preview;
mod report;
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
pub(crate) const APP_ID: &str = "io.github.ankurk91.Rollcopy";
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
        Eye,
        EyeOff,
        LayoutGrid,
        List,
        Github,
        Settings,
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
    let dirs = paths::app_dirs();
    worker::CachePaths {
        root: dirs.cache.clone(),
        state: dirs.state.clone(),
    }
}

fn main() {
    let paths = app_paths();
    log::init(&paths::app_dirs().log());
    applog!("Rollcopy {} starting", update::current_version());
    // Held until exit; the OS releases it even after a crash.
    let _instance = match instance::lock(&paths.state) {
        instance::Lock::Held => {
            match instance::activate_running() {
                Ok(()) => applog!("Rollcopy is already running; showed its window."),
                Err(e) => applog!("Rollcopy is already running (couldn't show it: {e})."),
            }
            return;
        }
        instance::Lock::Acquired(f) => Some(f),
        instance::Lock::Unavailable => None,
    };
    let worker = worker::spawn(paths.clone());
    let paths_at_exit = paths.clone();

    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(prefs::Prefs::load());
        view::bind_keys(cx);
        // App-wide actions; the main window handles Quit itself first so
        // it can save its state.
        cx.on_action(|_: &view::OpenPreferences, cx| prefs_window::open(cx));
        cx.on_action(|_: &view::ViewOnGitHub, cx| cx.open_url(update::REPO_URL));
        // The main window answers this itself, adding the phone model.
        cx.on_action(|_: &view::ReportIssue, cx| {
            cx.open_url(&report::DebugInfo::collect(None).issue_url())
        });
        cx.on_action(|_: &view::Quit, cx| cx.quit());
        // Session-only files (viewer images, opened files) go at quit.
        cx.on_app_quit(move |_| {
            paths_at_exit.tidy();
            async {}
        })
        .detach();

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
        #[expect(clippy::expect_used, reason = "nothing to run without a window")]
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
