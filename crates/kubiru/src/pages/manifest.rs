//! One object as YAML or JSON, with editing and deletion.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
};
use kubiru_core::kube::ObjectRef;
use kubiru_core::kube::manifest::{self, Format};
use serde_json::Value;

use crate::actions::{ApplyManifest, Back, MANIFEST};
use crate::pages::{NavEvent, confirm_delete, delete_object};
use crate::session::{Load, Session};
use crate::ui;

pub struct ManifestPage {
    session: Entity<Session>,
    object: ObjectRef,
    /// The object as last read from the server.
    original: Load<Value>,
    format: Format,
    editor: Entity<EditorState>,
    /// The editor text when the object was last loaded or saved.
    baseline: String,
    editing: bool,
    applying: bool,
    /// A problem with the edited text or the last apply.
    error: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<NavEvent> for ManifestPage {}

impl ManifestPage {
    pub fn new(
        session: Entity<Session>,
        object: ObjectRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(Format::Yaml.language())
                .line_number(true)
                .searchable(true)
                .soft_wrap(false)
        });
        let mut this = Self {
            session,
            object,
            original: Load::Idle,
            format: Format::Yaml,
            editor,
            baseline: String::new(),
            editing: false,
            applying: false,
            error: None,
            focus: cx.focus_handle(),
        };
        this.load(window, cx);
        this
    }

    pub fn object(&self) -> &ObjectRef {
        &self.object
    }

    /// Whether there are edits that would be lost by leaving.
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.editing && self.editor.read(cx).value().as_ref() != self.baseline
    }

    pub fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client() else {
            self.original = Load::Failed("Not connected to a cluster.".into());
            cx.notify();
            return;
        };
        self.original = Load::Loading;
        self.error = None;
        cx.notify();
        let object = self.object.clone();
        let task = cx.background_spawn(async move { client.get(&object) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(value) => this.show(value, window, cx),
                    Err(e) => this.original = Load::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Put a freshly read object in the editor and leave edit mode.
    fn show(&mut self, value: Value, window: &mut Window, cx: &mut Context<Self>) {
        let text = match manifest::render(&value, self.format) {
            Ok(text) => text,
            Err(e) => {
                self.original = Load::Failed(e.to_string());
                return;
            }
        };
        self.baseline = text.clone();
        self.editor
            .update(cx, |e, cx| e.set_value(text, window, cx));
        self.original = Load::Ready(value);
        self.editing = false;
        self.error = None;
    }

    fn set_format(&mut self, format: Format, window: &mut Window, cx: &mut Context<Self>) {
        if format == self.format {
            return;
        }
        let current = self.editor.read(cx).value().to_string();
        match manifest::convert(&current, self.format, format) {
            Ok(text) => {
                // Keep "dirty" meaning "differs from the server", in the new format.
                if let Some(original) = self.original.ready() {
                    self.baseline = manifest::render(original, format).unwrap_or_default();
                }
                self.format = format;
                self.editor
                    .update(cx, |e, cx| e.set_value(text, window, cx));
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Fix the manifest before switching format: {e}")),
        }
        cx.notify();
    }

    fn start_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = true;
        self.editor.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let baseline = self.baseline.clone();
        self.editor
            .update(cx, |e, cx| e.set_value(baseline, window, cx));
        self.editing = false;
        self.error = None;
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editing || self.applying {
            return;
        }
        let Some(original) = self.original.ready() else {
            return;
        };
        let text = self.editor.read(cx).value().to_string();
        let body = match manifest::parse(&text, self.format)
            .and_then(|v| manifest::prepare_update(original, v))
        {
            Ok(body) => body,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        self.applying = true;
        self.error = None;
        cx.notify();
        let object = self.object.clone();
        let task = cx.background_spawn(async move { client.replace(&object, &body) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.applying = false;
                match result {
                    Ok(value) => {
                        this.show(value, window, cx);
                        window.push_notification(
                            Notification::success(format!(
                                "Saved {} \u{201c}{}\u{201d}.",
                                this.object.kind.kind(),
                                this.object.name
                            )),
                            cx,
                        );
                    }
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        window.push_notification(
            format!("Copied {} to the clipboard.", self.format.label()),
            cx,
        );
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let page = cx.entity().downgrade();
        confirm_delete(&self.object, window, cx, move |object, window, cx| {
            let page = page.clone();
            delete_object(&session, object, window, cx, move |_, cx| {
                page.update(cx, |_, cx| cx.emit(NavEvent::Back)).ok();
            });
        });
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.original.ready().is_some();
        let format_switch = ButtonGroup::new("format")
            .outline()
            .small()
            .child(
                Button::new("yaml")
                    .label("YAML")
                    .selected(self.format == Format::Yaml),
            )
            .child(
                Button::new("json")
                    .label("JSON")
                    .selected(self.format == Format::Json),
            )
            .on_click(cx.listener(|this, selected: &Vec<usize>, window, cx| {
                let format = if selected.first() == Some(&1) {
                    Format::Json
                } else {
                    Format::Yaml
                };
                this.set_format(format, window, cx);
            }));

        let actions = if self.editing {
            h_flex()
                .gap_2()
                .child(
                    Button::new("discard")
                        .outline()
                        .label("Discard")
                        .disabled(self.applying)
                        .on_click(cx.listener(|this, _, window, cx| this.discard(window, cx))),
                )
                .child(
                    Button::new("apply")
                        .primary()
                        .icon(IconName::Save)
                        .label("Apply")
                        .loading(self.applying)
                        .tooltip(ui::with_shortcut("Apply changes to the cluster", "S"))
                        .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                )
        } else {
            h_flex()
                .gap_2()
                .child(
                    Button::new("copy")
                        .ghost()
                        .icon(IconName::Copy)
                        .tooltip("Copy to clipboard")
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, window, cx| this.copy(window, cx))),
                )
                .child(
                    Button::new("reload")
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .tooltip("Reload from the cluster")
                        .loading(self.original.is_loading())
                        .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                )
                .child(
                    Button::new("delete")
                        .outline()
                        .danger()
                        .icon(IconName::Trash)
                        .label("Delete")
                        .disabled(!ready)
                        .on_click(cx.listener(|this, _, window, cx| this.delete(window, cx))),
                )
                .child(
                    Button::new("edit")
                        .primary()
                        .icon(IconName::Pencil)
                        .label("Edit")
                        .disabled(!ready)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.start_editing(window, cx)),
                        ),
                )
        };
        h_flex().gap_3().child(format_switch).child(actions)
    }
}

impl Focusable for ManifestPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ManifestPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let object = &self.object;
        let subtitle: SharedString = match &object.namespace {
            Some(ns) => format!("{} in {ns}", object.kind.kind()).into(),
            None => object.kind.kind().into(),
        };
        let back = Button::new("back")
            .ghost()
            .icon(IconName::ArrowLeft)
            .tooltip("Back (Esc)")
            .on_click(cx.listener(|_, _, _, cx| cx.emit(NavEvent::Back)));

        let body = match &self.original {
            Load::Failed(error) => {
                let retry = Button::new("retry")
                    .outline()
                    .icon(IconName::RefreshCw)
                    .label("Try again")
                    .on_click(cx.listener(|this, _, window, cx| this.load(window, cx)));
                ui::empty_state(
                    IconName::TriangleAlert,
                    "Could not load the object",
                    error.clone(),
                    Some(retry.into_any_element()),
                    cx,
                )
                .into_any_element()
            }
            _ => Editor::new(&self.editor)
                .readonly(!self.editing || self.applying)
                .size_full()
                .into_any_element(),
        };

        let toolbar = self.toolbar(cx).into_any_element();
        v_flex()
            .id("manifest-page")
            .key_context(MANIFEST)
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &Back, _, cx| cx.emit(NavEvent::Back)))
            .on_action(cx.listener(|this, _: &ApplyManifest, window, cx| this.apply(window, cx)))
            .size_full()
            .gap_4()
            .p_6()
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(back)
                    .child(div().flex_1().min_w_0().child(ui::page_header(
                        object.name.clone(),
                        Some(subtitle),
                        toolbar,
                        cx,
                    ))),
            )
            .children(self.editing.then(|| {
                Alert::info(
                    "editing-hint",
                    "You are editing the live object. Apply sends the whole manifest to the cluster; the name, namespace and kind cannot be changed.",
                )
                .small()
            }))
            .children(self.error.clone().map(|e| Alert::error("manifest-error", e).title("Not applied")))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded(cx.theme().radius)
                    .child(body),
            )
    }
}
