//! The views shown in the main area of the window.

pub mod resources;

use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Entity, Window};
use kubiru_core::kube::{ObjectRef, ResourceKind};

use crate::session::Session;

/// Where the main area can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Overview,
    Resources(ResourceKind),
    Manifest,
    Clusters,
    Settings,
}

/// Requests from a page to the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavEvent {
    Go(Route),
    Open(ObjectRef),
    Back,
    AddCluster,
}

/// Ask before deleting `object`; run `on_confirm` if the user agrees.
pub fn confirm_delete(
    object: &ObjectRef,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(ObjectRef, &mut Window, &mut App) + 'static,
) {
    let object = object.clone();
    let on_confirm = Rc::new(on_confirm);
    let title = format!(
        "Delete {} \u{201c}{}\u{201d}?",
        object.kind.kind().to_lowercase(),
        object.name
    );
    let description = match &object.namespace {
        Some(ns) => format!(
            "It will be removed from the {ns} namespace together with anything it owns. This cannot be undone."
        ),
        None => "It will be removed from the cluster together with anything it owns. This cannot be undone."
            .to_owned(),
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let object = object.clone();
        let on_confirm = on_confirm.clone();
        alert
            .title(title.clone())
            .description(description.clone())
            .ok_text("Delete")
            .ok_variant(ButtonVariant::Danger)
            .cancel_text("Cancel")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                on_confirm(object.clone(), window, cx);
                true
            })
    });
}

/// Delete `object` in the background and report the outcome.
pub fn delete_object(
    session: &Entity<Session>,
    object: ObjectRef,
    window: &mut Window,
    cx: &mut App,
    on_deleted: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    let Some(client) = session.read(cx).client() else {
        window.push_notification(Notification::error("Not connected to a cluster."), cx);
        return;
    };
    let label = format!("{} \u{201c}{}\u{201d}", object.kind.kind(), object.name);
    let task = cx.background_spawn(async move { client.delete(&object) });
    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            cx.update(|window, cx| match result {
                Ok(()) => {
                    window.push_notification(
                        Notification::success(format!("{label} is being deleted.")),
                        cx,
                    );
                    on_deleted(window, cx);
                }
                Err(e) => window.push_notification(
                    Notification::error(e.to_string()).title(format!("Could not delete {label}")),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}
