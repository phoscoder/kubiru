//! Kubiru: a native desktop client for Kubernetes, built with GPUI Kit.

mod actions;
mod appearance;
mod session;

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Bounds, Context, FontWeight, IntoElement, ParentElement, Render,
    SharedString, Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, div, px, size,
};
use kubiru_core::store::{self, Store, keys};

use crate::appearance::Appearance;

fn main() {
    let data_path = store::default_path();
    let opened = match &data_path {
        Some(path) => Store::open(path).map_err(|e| format!("{e}\n\nDatabase: {}", path.display())),
        None => Err("Could not determine a data directory for this user.".to_owned()),
    };

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            actions::bind(cx);
            cx.on_action(|_: &actions::Quit, cx| cx.quit());
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = window_options(cx);
            let (appearance, message) = match opened {
                Ok(store) => {
                    let appearance = store
                        .preference(keys::THEME)
                        .ok()
                        .flatten()
                        .map(|k| Appearance::from_key(&k))
                        .unwrap_or_default();
                    (appearance, None)
                }
                Err(message) => (Appearance::System, Some(message)),
            };
            let result = gpui_kit::open_window(options, cx, move |window, cx| {
                appearance.apply(Some(window), cx);
                cx.new(|_| Message {
                    title: if message.is_some() {
                        "Kubiru could not start"
                    } else {
                        "Kubiru"
                    }
                    .into(),
                    message: message.unwrap_or_default().into(),
                })
            });
            if let Err(e) = result {
                // There is no window to show this in.
                #[allow(clippy::print_stderr)]
                {
                    eprintln!("kubiru: could not open a window: {e}");
                }
                cx.quit();
            }
            cx.activate(true);
        });
}

fn window_options(cx: &mut App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(820.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Kubiru".into()),
            ..Default::default()
        }),
        app_id: Some("dev.kubiru.Kubiru".into()),
        window_min_size: Some(size(px(900.), px(560.))),
        ..Default::default()
    }
}

/// A centred title and message.
struct Message {
    title: SharedString,
    message: SharedString,
}

impl Render for Message {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_8()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .max_w(px(560.))
                    .text_center()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.message.clone()),
            )
    }
}
