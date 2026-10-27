//! Health at a glance for the active cluster.

use chrono::Utc;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px, relative,
};
use kubiru_core::kube::overview::{Overview, summarize};
use kubiru_core::kube::table::Tone;
use kubiru_core::kube::{KubeError, ResourceKind, ServerVersion};

use crate::pages::{NavEvent, Route};
use crate::session::{Load, Session, SessionEvent, client_and_namespace};
use crate::ui;

#[derive(Clone, Debug)]
struct Snapshot {
    version: ServerVersion,
    overview: Overview,
}

pub struct OverviewPage {
    session: Entity<Session>,
    data: Load<Snapshot>,
    request: u64,
    _subscription: Subscription,
}

impl EventEmitter<NavEvent> for OverviewPage {}

impl OverviewPage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| {
            if *event == SessionEvent::ContextChanged {
                this.reload(cx);
            }
        });
        let mut this = Self {
            session,
            data: Load::Idle,
            request: 0,
            _subscription: subscription,
        };
        this.reload(cx);
        this
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (client, namespace, _) = client_and_namespace(&self.session, cx);
        self.request += 1;
        let Some(client) = client else {
            self.data = Load::Idle;
            cx.notify();
            return;
        };
        let request = self.request;
        self.data = Load::Loading;
        cx.notify();
        let task = cx.background_spawn(async move {
            let ns = namespace.as_deref();
            let result: Result<Snapshot, KubeError> = (|| {
                let version = client.verify()?;
                let nodes = client.list(ResourceKind::Nodes, None)?;
                let deployments = client.list(ResourceKind::Deployments, ns)?;
                let pods = client.list(ResourceKind::Pods, ns)?;
                Ok(Snapshot {
                    version,
                    overview: summarize(&nodes, &deployments, &pods, Utc::now()),
                })
            })();
            result
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.request == request {
                    this.data = match result {
                        Ok(snapshot) => Load::Ready(snapshot),
                        Err(e) => Load::Failed(e.to_string()),
                    };
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn render_snapshot(&self, snapshot: &Snapshot, cx: &mut Context<Self>) -> impl IntoElement {
        let o = &snapshot.overview;
        let pods = o.pods;
        let total = pods.total().max(1) as f32;
        let segment = |count: usize, tone: Tone, cx: &App| {
            div()
                .h_full()
                .w(relative(count as f32 / total))
                .bg(ui::tone_color(tone, cx))
        };
        let legend = |label: &str, count: usize, tone: Tone, cx: &App| {
            ui::status(format!("{count} {label}"), tone, cx)
        };
        let health_card = |id: &'static str,
                           title: &str,
                           health: kubiru_core::kube::overview::Health,
                           route: Route,
                           cx: &mut Context<Self>| {
            div()
                .id(id)
                .flex_1()
                .cursor_pointer()
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(NavEvent::Go(route))))
                .child(ui::stat_card(
                    title.to_owned(),
                    format!("{}/{}", health.healthy, health.total),
                    ui::status(
                        if health.healthy == health.total {
                            "All healthy".to_owned()
                        } else {
                            format!("{} need attention", health.total - health.healthy)
                        },
                        health.tone(),
                        cx,
                    ),
                    cx,
                ))
        };

        let problems = if o.problems.is_empty() {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No failing pods. \u{1f389}")
                .into_any_element()
        } else {
            v_flex()
                .children(o.problems.iter().enumerate().map(|(i, p)| {
                    let object = p.object.clone();
                    h_flex()
                        .id(("problem", i))
                        .justify_between()
                        .gap_4()
                        .px_3()
                        .py_2()
                        .rounded(cx.theme().radius)
                        .cursor_pointer()
                        .hover(|s| s.bg(cx.theme().accent))
                        .on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(NavEvent::Open(object.clone()))),
                        )
                        .child(
                            v_flex()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(p.object.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(p.object.namespace.clone().unwrap_or_default()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_4()
                                .flex_shrink_0()
                                .child(ui::status(p.status.clone(), Tone::Bad, cx))
                                .child(
                                    div()
                                        .w(px(90.))
                                        .text_right()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("{} restarts", p.restarts)),
                                ),
                        )
                }))
                .into_any_element()
        };

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .child(health_card(
                        "nodes-card",
                        "Nodes ready",
                        o.nodes,
                        Route::Resources(ResourceKind::Nodes),
                        cx,
                    ))
                    .child(health_card(
                        "deployments-card",
                        "Deployments available",
                        o.deployments,
                        Route::Resources(ResourceKind::Deployments),
                        cx,
                    ))
                    .child(div().flex_1().child(ui::stat_card(
                        "Kubernetes version",
                        if snapshot.version.git_version.is_empty() {
                            "unknown".to_owned()
                        } else {
                            snapshot.version.git_version.clone()
                        },
                        div().child(snapshot.version.platform.clone()),
                        cx,
                    ))),
            )
            .child(
                v_flex()
                    .gap_3()
                    .p_4()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .justify_between()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{} pods", pods.total())),
                            )
                            .child(Button::new("view-pods").ghost().label("View all").on_click(
                                cx.listener(|_, _, _, cx| {
                                    cx.emit(NavEvent::Go(Route::Resources(ResourceKind::Pods)))
                                }),
                            )),
                    )
                    .child(
                        h_flex()
                            .h(px(8.))
                            .w_full()
                            .rounded_full()
                            .overflow_hidden()
                            .bg(cx.theme().muted)
                            .child(segment(pods.running, Tone::Good, cx))
                            .child(segment(pods.pending, Tone::Warning, cx))
                            .child(segment(pods.failed, Tone::Bad, cx))
                            .child(segment(pods.succeeded + pods.unknown, Tone::Neutral, cx)),
                    )
                    .child(
                        h_flex()
                            .gap_6()
                            .flex_wrap()
                            .text_sm()
                            .child(legend("running", pods.running, Tone::Good, cx))
                            .child(legend("pending", pods.pending, Tone::Warning, cx))
                            .child(legend("failed", pods.failed, Tone::Bad, cx))
                            .child(legend("completed", pods.succeeded, Tone::Neutral, cx)),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Pods needing attention"),
                    )
                    .child(problems),
            )
    }
}

impl Render for OverviewPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let cluster = session.active_cluster().cloned();
        let connection_error = match session.connection() {
            Load::Failed(e) => Some(e.clone()),
            _ => None,
        };
        let scope: SharedString = match session.namespace() {
            Some(ns) => format!("Workloads in {ns}").into(),
            None => "Workloads in all namespaces".into(),
        };

        let Some(cluster) = cluster else {
            let add = Button::new("add-first-cluster")
                .primary()
                .icon(IconName::Plus)
                .label("Add a cluster")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(NavEvent::AddCluster)));
            return ui::empty_state(
                IconName::Ship,
                "Welcome to Kubiru",
                "Kubiru is a fast, native window into your Kubernetes clusters. Add a cluster with its API server address and a bearer token to get started.",
                Some(add.into_any_element()),
                cx,
            )
            .into_any_element();
        };

        let refresh = Button::new("refresh-overview")
            .ghost()
            .icon(IconName::RefreshCw)
            .tooltip(ui::with_shortcut("Refresh", "R"))
            .loading(self.data.is_loading())
            .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));

        let body = match (&self.data, connection_error) {
            (_, Some(error)) => {
                let retry = Button::new("reconnect")
                    .outline()
                    .icon(IconName::RefreshCw)
                    .label("Try again")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| s.reconnect(cx))
                    }));
                ui::empty_state(
                    IconName::Unplug,
                    "Could not connect",
                    error,
                    Some(retry.into_any_element()),
                    cx,
                )
                .into_any_element()
            }
            (Load::Ready(snapshot), _) => self
                .render_snapshot(&snapshot.clone(), cx)
                .into_any_element(),
            (Load::Failed(error), _) => {
                let retry = Button::new("retry")
                    .outline()
                    .icon(IconName::RefreshCw)
                    .label("Try again")
                    .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));
                ui::empty_state(
                    IconName::TriangleAlert,
                    "Could not load the overview",
                    error.clone(),
                    Some(retry.into_any_element()),
                    cx,
                )
                .into_any_element()
            }
            _ => ui::empty_state(
                IconName::Plug,
                "Loading\u{2026}",
                "Asking the cluster for its current state.",
                None,
                cx,
            )
            .into_any_element(),
        };

        v_flex()
            .id("overview-page")
            .size_full()
            .overflow_y_scroll()
            .gap_6()
            .p_6()
            .child(ui::page_header(
                cluster.name.clone(),
                Some(format!("{} \u{b7} {scope}", cluster.api_url).into()),
                refresh,
                cx,
            ))
            .child(body)
            .into_any_element()
    }
}
