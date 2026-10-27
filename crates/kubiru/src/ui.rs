//! Small presentation pieces shared by the pages.

use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, Div, FontWeight, Hsla, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use kubiru_core::kube::table::Tone;

/// The theme colour for a health tone.
pub fn tone_color(tone: Tone, cx: &App) -> Hsla {
    let theme = cx.theme();
    match tone {
        Tone::Neutral => theme.muted_foreground,
        Tone::Good => theme.success,
        Tone::Warning => theme.warning,
        Tone::Bad => theme.danger,
    }
}

/// A coloured dot followed by text; the dot is not the only signal, the
/// text always carries the meaning too.
pub fn status(text: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    h_flex()
        .gap_2()
        .min_w_0()
        .child(
            div()
                .flex_shrink_0()
                .size(px(8.))
                .rounded_full()
                .bg(tone_color(tone, cx)),
        )
        .child(div().truncate().child(text.into()))
}

/// Title and optional subtitle at the top of a page, with actions on the right.
pub fn page_header(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    actions: impl IntoElement,
    cx: &App,
) -> Div {
    h_flex()
        .w_full()
        .justify_between()
        .items_start()
        .gap_4()
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.into()),
                )
                .children(subtitle.map(|s| {
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(s)
                })),
        )
        .child(h_flex().flex_shrink_0().gap_2().child(actions))
}

/// A centred message for empty, loading-failed or not-yet-configured views.
pub fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
    action: Option<gpui_kit::AnyElement>,
    cx: &App,
) -> Div {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_8()
        .child(
            div()
                .p_3()
                .rounded_full()
                .bg(cx.theme().muted)
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(icon).large()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(
            div()
                .max_w(px(460.))
                .text_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(message.into()),
        )
        .children(action)
}

/// A labelled number on the overview.
pub fn stat_card(
    title: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: Div,
    cx: &App,
) -> Div {
    v_flex()
        .flex_1()
        .min_w(px(180.))
        .gap_2()
        .p_4()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(title.into()),
        )
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(value.into()),
        )
        .child(detail.text_sm().text_color(cx.theme().muted_foreground))
}

/// A tooltip that names its keyboard shortcut the way the platform writes it:
/// `with_shortcut("Refresh", "R")` is "Refresh (⌘R)" on macOS and
/// "Refresh (Ctrl+R)" elsewhere.
pub fn with_shortcut(text: &str, key: &str) -> SharedString {
    let modifier = if cfg!(target_os = "macos") {
        "\u{2318}"
    } else {
        "Ctrl+"
    };
    format!("{text} ({modifier}{key})").into()
}
