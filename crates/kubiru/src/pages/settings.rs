//! Preferences and information about the app.

use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, h_flex, v_flex};
use gpui_kit::{
    Context, Entity, EventEmitter, FontWeight, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div,
};
use kubiru_core::store::keys;

use crate::appearance::Appearance;
use crate::session::Session;
use crate::ui;

pub struct SettingsPage {
    session: Entity<Session>,
    appearance: Appearance,
    data_path: Option<SharedString>,
}

/// The user picked another appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppearanceChanged(pub Appearance);

impl EventEmitter<AppearanceChanged> for SettingsPage {}

impl SettingsPage {
    pub fn new(
        session: Entity<Session>,
        appearance: Appearance,
        data_path: Option<SharedString>,
    ) -> Self {
        Self {
            session,
            appearance,
            data_path,
        }
    }

    pub fn appearance(&self) -> Appearance {
        self.appearance
    }

    pub fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        if appearance == self.appearance {
            return;
        }
        self.appearance = appearance;
        self.session
            .read(cx)
            .registry()
            .with_store(|s| s.set_preference(keys::THEME, appearance.key()))
            .ok();
        cx.emit(AppearanceChanged(appearance));
        cx.notify();
    }
}

fn section(
    title: &'static str,
    description: &'static str,
    control: impl IntoElement,
    cx: &Context<SettingsPage>,
) -> impl IntoElement {
    h_flex()
        .justify_between()
        .items_start()
        .gap_6()
        .py_4()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .gap_1()
                .max_w(gpui_kit::px(420.))
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(div().flex_shrink_0().child(control))
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.appearance;
        let appearance = ButtonGroup::new("appearance")
            .outline()
            .children(
                Appearance::ALL
                    .into_iter()
                    .map(|a| Button::new(a.key()).label(a.label()).selected(a == current)),
            )
            .on_click(cx.listener(|this, selected: &Vec<usize>, _, cx| {
                if let Some(appearance) = selected.first().and_then(|&i| Appearance::ALL.get(i)) {
                    this.set_appearance(*appearance, cx);
                }
            }));
        let muted = cx.theme().muted_foreground;
        let mono = |text: SharedString| {
            div()
                .text_sm()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(muted)
                .child(text)
        };

        v_flex()
            .size_full()
            .gap_2()
            .p_6()
            .child(ui::page_header("Settings", None, div(), cx))
            .child(
                v_flex()
                    .max_w(gpui_kit::px(820.))
                    .child(section("Appearance", "Follow the system, or always use a light or dark theme.", appearance, cx))
                    .child(section(
                        "Credentials",
                        "Cluster tokens are stored in the operating system keychain and are never written to disk by Kubiru.",
                        mono(kubiru_core::vault::KEYCHAIN_SERVICE.into()),
                        cx,
                    ))
                    .child(section(
                        "Local data",
                        "Cluster addresses, CA certificates and preferences are kept in this database, readable only by your user.",
                        mono(self.data_path.clone().unwrap_or_else(|| "in memory".into())),
                        cx,
                    ))
                    .child(section(
                        "About",
                        "Kubiru is free software under the MIT licence.",
                        mono(format!("Kubiru {}", env!("CARGO_PKG_VERSION")).into()),
                        cx,
                    )),
            )
    }
}
