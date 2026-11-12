//! Unspool desktop app (binary: `unspool`).

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

const APP_NAME: &str = "Unspool";

struct HelloWindow;

impl Render for HelloWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .gap_3()
            .size_full()
            .items_center()
            .justify_center()
            .child(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION")))
            .child("Plug in your iPhone with a cable.")
            .child(
                Button::new("hello")
                    .primary()
                    .label("Hello")
                    .on_click(|_, _, _| eprintln!("hello clicked")),
            )
    }
}

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(900.), px(600.)), cx)),
            app_id: Some("dev.unspool.Unspool".into()),
            titlebar: Some(TitlebarOptions {
                title: Some(APP_NAME.into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| HelloWindow))
            .expect("failed to open window");
    });
}
