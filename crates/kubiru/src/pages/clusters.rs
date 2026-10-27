//! The saved clusters.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, StatefulInteractiveElement as _, Styled, Subscription,
    Window, div,
};
use kubiru_core::{Cluster, ClusterId, TlsVerification};

use crate::pages::NavEvent;
use crate::session::{Session, SessionEvent};
use crate::ui;

/// Asks the workspace to open the cluster form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditCluster(pub Option<Cluster>);

pub struct ClustersPage {
    session: Entity<Session>,
    _subscription: Subscription,
}

impl EventEmitter<NavEvent> for ClustersPage {}
impl EventEmitter<EditCluster> for ClustersPage {}

impl ClustersPage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |_, _, event, cx| {
            if *event == SessionEvent::ClustersChanged {
                cx.notify();
            }
        });
        Self {
            session,
            _subscription: subscription,
        }
    }

    fn delete(&mut self, cluster: Cluster, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        let name = cluster.name.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let session = session.clone();
            let id = cluster.id;
            let name = name.clone();
            alert
                .title(format!("Remove \u{201c}{name}\u{201d}?"))
                .description(
                    "Kubiru will forget this cluster and delete its token from the system keychain. Nothing in the cluster itself is changed.",
                )
                .ok_text("Remove")
                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    remove_cluster(&session, id, name.clone(), window, cx);
                    true
                })
        });
    }

    fn render_cluster(
        &self,
        ix: usize,
        cluster: &Cluster,
        active: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (tls_label, tls_tag) = match &cluster.tls {
            _ if cluster.api_url.scheme() == "http" => ("Local, no TLS", Tag::secondary()),
            TlsVerification::System => ("TLS verified", Tag::success()),
            TlsVerification::CustomCa(_) => ("Custom CA", Tag::info()),
            TlsVerification::Insecure => ("TLS not verified", Tag::danger()),
        };
        let edit_target = cluster.clone();
        let delete_target = cluster.clone();
        let id = cluster.id;
        h_flex()
            .id(("cluster", ix))
            .gap_4()
            .p_4()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(if active {
                cx.theme().primary
            } else {
                cx.theme().border
            })
            .child(
                div()
                    .flex_shrink_0()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .child(gpui_kit::component::Icon::new(IconName::Ship)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(cluster.name.clone()),
                            )
                            .children(active.then(|| Tag::primary().small().child("Active")))
                            .child(tls_tag.outline().small().child(tls_label)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(cluster.api_url.to_string()),
                    )
                    .children(
                        (!cluster.description.is_empty())
                            .then(|| div().text_sm().child(cluster.description.clone())),
                    ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .children((!active).then(|| {
                        Button::new(("connect", ix))
                            .outline()
                            .small()
                            .label("Connect")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session.update(cx, |s, cx| s.select_cluster(id, cx));
                                cx.emit(NavEvent::Go(crate::pages::Route::Overview));
                            }))
                    }))
                    .child(
                        Button::new(("edit", ix))
                            .ghost()
                            .small()
                            .icon(IconName::Pencil)
                            .tooltip("Edit")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(EditCluster(Some(edit_target.clone())))
                            })),
                    )
                    .child(
                        Button::new(("remove", ix))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .tooltip("Remove")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.delete(delete_target.clone(), window, cx)
                            })),
                    ),
            )
    }
}

/// Delete a cluster in the background (the keychain can block) and report it.
fn remove_cluster(
    session: &Entity<Session>,
    id: ClusterId,
    name: String,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) {
    let registry = session.read(cx).registry().clone();
    let session = session.clone();
    let task = cx.background_spawn(async move { registry.delete(id) });
    window
        .spawn(cx, async move |cx| {
            let result = task.await;
            cx.update(|window, cx| {
                match result {
                    Ok(()) => window.push_notification(
                        Notification::success(format!("Removed \u{201c}{name}\u{201d}.")),
                        cx,
                    ),
                    Err(e) => window.push_notification(
                        Notification::error(e.to_string())
                            .title(format!("Could not remove \u{201c}{name}\u{201d}")),
                        cx,
                    ),
                }
                session.update(cx, |s, cx| s.reload_clusters(None, cx));
            })
            .ok();
        })
        .detach();
}

impl Render for ClustersPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let clusters = session.clusters().to_vec();
        let active = session.active_cluster().map(|c| c.id);
        let error = session.clusters_error().map(str::to_owned);

        let add = Button::new("add-cluster")
            .primary()
            .icon(IconName::Plus)
            .label("Add cluster")
            .tooltip(ui::with_shortcut("Add cluster", "N"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(EditCluster(None))));

        let body = if let Some(error) = error {
            ui::empty_state(
                IconName::TriangleAlert,
                "Could not read saved clusters",
                error,
                None,
                cx,
            )
            .into_any_element()
        } else if clusters.is_empty() {
            ui::empty_state(
                IconName::Ship,
                "No clusters yet",
                "Add a cluster with its API server address and a bearer token. The token is kept in your system keychain.",
                None,
                cx,
            )
            .into_any_element()
        } else {
            let mut cards = Vec::with_capacity(clusters.len());
            for (ix, c) in clusters.iter().enumerate() {
                cards.push(
                    self.render_cluster(ix, c, Some(c.id) == active, cx)
                        .into_any_element(),
                );
            }
            v_flex().gap_3().children(cards).into_any_element()
        };

        v_flex()
            .id("clusters-page")
            .size_full()
            .overflow_y_scroll()
            .gap_6()
            .p_6()
            .child(ui::page_header(
                "Clusters",
                Some("Connections are stored locally; tokens live in the system keychain.".into()),
                add,
                cx,
            ))
            .child(body)
    }
}
