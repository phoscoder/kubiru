//! The window's content: sidebar, top bar and the current page.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarGroup, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, px,
};
use kubiru_core::kube::{Category, ObjectRef, ResourceKind};
use kubiru_core::store::keys;

use crate::actions::*;
use crate::appearance::Appearance;
use crate::pages::cluster_form::{ClusterForm, FormEvent};
use crate::pages::clusters::{ClustersPage, EditCluster};
use crate::pages::manifest::ManifestPage;
use crate::pages::overview::OverviewPage;
use crate::pages::resources::ResourcesPage;
use crate::pages::settings::{AppearanceChanged, SettingsPage};
use crate::pages::{NavEvent, Route};
use crate::session::{Load, Session, SessionEvent};

/// An entry of the namespace switcher. `None` is "all namespaces".
#[derive(Clone, Debug, PartialEq, Eq)]
struct NamespaceItem {
    title: SharedString,
    value: Option<String>,
}

impl SearchableListItem for NamespaceItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

type NamespaceSelect = SelectState<SearchableVec<NamespaceItem>>;

pub struct Workspace {
    session: Entity<Session>,
    route: Route,
    /// Where "Back" returns to from the manifest view.
    previous: Route,
    sidebar_collapsed: bool,
    overview: Entity<OverviewPage>,
    resources: Entity<ResourcesPage>,
    clusters: Entity<ClustersPage>,
    settings: Entity<SettingsPage>,
    manifest: Option<Entity<ManifestPage>>,
    namespace: Entity<NamespaceSelect>,
    focus: FocusHandle,
    _manifest_subscription: Option<Subscription>,
    _form_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(
        session: Entity<Session>,
        appearance: Appearance,
        data_path: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let overview = cx.new(|cx| OverviewPage::new(session.clone(), cx));
        let resources = cx.new(|cx| ResourcesPage::new(session.clone(), window, cx));
        let clusters = cx.new(|cx| ClustersPage::new(session.clone(), cx));
        let settings = cx.new(|_| SettingsPage::new(session.clone(), appearance, data_path));
        let namespace = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(namespace_items(&session, cx)),
                None,
                window,
                cx,
            )
            .searchable(true)
        });

        let mut subscriptions = vec![
            cx.subscribe_in(&session, window, |this, _, event, window, cx| {
                if *event == SessionEvent::ContextChanged {
                    this.sync_namespaces(window, cx);
                    // A different cluster may not have the open object.
                    if this.route == Route::Manifest && this.manifest_cluster_changed(cx) {
                        this.close_manifest(window, cx);
                    }
                }
                cx.notify();
            }),
            cx.subscribe(
                &namespace,
                |this, _, event: &SelectEvent<SearchableVec<NamespaceItem>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let value = value.clone().flatten();
                    this.session
                        .update(cx, |s, cx| s.select_namespace(value, cx));
                },
            ),
            cx.subscribe_in(
                &settings,
                window,
                |_, _, event: &AppearanceChanged, window, cx| {
                    event.0.apply(Some(window), cx);
                },
            ),
            cx.subscribe_in(
                &clusters,
                window,
                |this, _, event: &EditCluster, window, cx| {
                    this.open_cluster_form(event.0.clone(), window, cx);
                },
            ),
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.settings.read(cx).appearance() == Appearance::System {
                    Appearance::System.apply(Some(window), cx);
                }
            }),
        ];
        subscriptions.push(cx.subscribe_in(&overview, window, Self::on_nav));
        subscriptions.push(cx.subscribe_in(&resources, window, Self::on_nav));
        subscriptions.push(cx.subscribe_in(&clusters, window, Self::on_nav));

        let sidebar_collapsed = session
            .read(cx)
            .registry()
            .with_store(|s| s.preference(keys::SIDEBAR_COLLAPSED))
            .ok()
            .flatten()
            .is_some_and(|v| v == "true");

        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = Self {
            session,
            route: Route::Overview,
            previous: Route::Overview,
            sidebar_collapsed,
            overview,
            resources,
            clusters,
            settings,
            manifest: None,
            namespace,
            focus,
            _manifest_subscription: None,
            _form_subscription: None,
            _subscriptions: subscriptions,
        };
        this.sync_namespaces(window, cx);
        this
    }

    #[cfg(test)]
    pub fn route(&self) -> Route {
        self.route
    }

    #[cfg(test)]
    pub fn sidebar_collapsed(&self) -> bool {
        self.sidebar_collapsed
    }

    /// Show `route`, asking first if that would lose unsaved edits.
    pub fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if route == self.route {
            return;
        }
        if self.route == Route::Manifest
            && self
                .manifest
                .as_ref()
                .is_some_and(|m| m.read(cx).is_dirty(cx))
        {
            let this = cx.entity().downgrade();
            window.open_alert_dialog(cx, move |alert, _, _| {
                let this = this.clone();
                alert
                    .title("Discard your changes?")
                    .description(
                        "The manifest has edits that have not been applied to the cluster.",
                    )
                    .ok_text("Discard")
                    .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                    .cancel_text("Keep editing")
                    .show_cancel(true)
                    .on_ok(move |_, window, cx| {
                        this.update(cx, |this, cx| this.go(route, window, cx)).ok();
                        true
                    })
            });
            return;
        }
        self.go(route, window, cx);
    }

    fn go(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if route != Route::Manifest {
            self.manifest = None;
            self._manifest_subscription = None;
        }
        match route {
            Route::Resources(kind) => self.resources.update(cx, |r, cx| r.show(kind, cx)),
            Route::Overview => self.overview.update(cx, |o, cx| o.reload(cx)),
            _ => {}
        }
        self.route = route;
        // Put the keyboard where the page's main content is.
        let focus = match route {
            Route::Resources(_) => self.resources.read(cx).table_focus_handle(cx),
            _ => self.focus.clone(),
        };
        focus.focus(window, cx);
        cx.notify();
    }

    fn open(&mut self, object: ObjectRef, window: &mut Window, cx: &mut Context<Self>) {
        let page = cx.new(|cx| ManifestPage::new(self.session.clone(), object, window, cx));
        self._manifest_subscription = Some(cx.subscribe_in(&page, window, Self::on_nav));
        page.read(cx).focus_handle(cx).focus(window, cx);
        if self.route != Route::Manifest {
            self.previous = self.route;
        }
        self.manifest = Some(page);
        self.route = Route::Manifest;
        cx.notify();
    }

    fn close_manifest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.previous;
        self.navigate(back, window, cx);
        // Coming back from a deletion or edit, show fresh data.
        if let Route::Resources(_) = back {
            self.resources.update(cx, |r, cx| r.reload(cx));
        }
    }

    fn manifest_cluster_changed(&self, cx: &App) -> bool {
        // Whatever the change, the loaded object belongs to the previous context.
        self.manifest.is_some() && self.session.read(cx).connection().is_loading()
    }

    fn on_nav<T>(
        &mut self,
        _: &Entity<T>,
        event: &NavEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            NavEvent::Go(route) => self.navigate(*route, window, cx),
            NavEvent::Open(object) => self.open(object.clone(), window, cx),
            NavEvent::Back => self.close_manifest(window, cx),
            NavEvent::AddCluster => self.open_cluster_form(None, window, cx),
        }
    }

    pub fn open_cluster_form(
        &mut self,
        cluster: Option<kubiru_core::Cluster>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = cx.new(|cx| ClusterForm::new(self.session.clone(), cluster, window, cx));
        self._form_subscription =
            Some(
                cx.subscribe_in(&form, window, |this, _, event, window, cx| {
                    match event {
                        FormEvent::Saved(id) => {
                            window.close_dialog(cx);
                            this.session
                                .update(cx, |s, cx| s.reload_clusters(Some(*id), cx));
                            window.push_notification(
                                gpui_kit::component::notification::Notification::success(
                                    "Cluster saved.",
                                ),
                                cx,
                            );
                            this.navigate(Route::Overview, window, cx);
                        }
                        FormEvent::Cancelled => window.close_dialog(cx),
                    }
                    this._form_subscription = None;
                }),
            );
        let title = form.read(cx).title();
        let first_field = form.read(cx).first_field(cx);
        first_field.focus(window, cx);
        window.open_dialog(cx, move |dialog, _, _| {
            dialog.title(title).w(px(580.)).child(form.clone())
        });
    }

    /// Rebuild the namespace switcher from the session.
    fn sync_namespaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = namespace_items(&self.session, cx);
        let selected = self.session.read(cx).namespace().map(str::to_owned);
        self.namespace.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(items), window, cx);
            state.set_selected_value(&selected, window, cx);
        });
    }

    fn set_sidebar_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.sidebar_collapsed = collapsed;
        let value = if collapsed { "true" } else { "false" };
        self.session
            .read(cx)
            .registry()
            .with_store(|s| s.set_preference(keys::SIDEBAR_COLLAPSED, value))
            .ok();
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.route {
            Route::Overview => self.overview.update(cx, |o, cx| o.reload(cx)),
            Route::Resources(_) => self.resources.update(cx, |r, cx| r.reload(cx)),
            Route::Manifest => {
                if let Some(m) = &self.manifest {
                    m.update(cx, |m, cx| m.load(window, cx));
                }
            }
            Route::Clusters => self.session.update(cx, |s, cx| s.reload_clusters(None, cx)),
            Route::Settings => {}
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let route = self.route;
        let current_kind = match (route, &self.manifest) {
            (Route::Resources(kind), _) => Some(kind),
            (Route::Manifest, Some(m)) => Some(m.read(cx).object().kind),
            _ => None,
        };
        let item = |label: &'static str,
                    icon: IconName,
                    target: Route,
                    active: bool,
                    cx: &mut Context<Self>| {
            SidebarMenuItem::new(label)
                .icon(icon)
                .active(active)
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(target, window, cx)))
        };

        let mut groups: Vec<SidebarGroup<SidebarMenu>> =
            vec![
                SidebarGroup::new("General").child(SidebarMenu::new().child(item(
                    "Overview",
                    IconName::LayoutDashboard,
                    Route::Overview,
                    route == Route::Overview,
                    cx,
                ))),
            ];
        for category in Category::ALL {
            let menu = SidebarMenu::new().children(category.kinds().map(|kind| {
                item(
                    kind.title(),
                    kind_icon(kind),
                    Route::Resources(kind),
                    current_kind == Some(kind),
                    cx,
                )
            }));
            groups.push(SidebarGroup::new(category.title()).child(menu));
        }
        groups.push(
            SidebarGroup::new("Manage").child(
                SidebarMenu::new()
                    .child(item(
                        "Clusters",
                        IconName::Server,
                        Route::Clusters,
                        route == Route::Clusters,
                        cx,
                    ))
                    .child(item(
                        "Settings",
                        IconName::Settings,
                        Route::Settings,
                        route == Route::Settings,
                        cx,
                    )),
            ),
        );

        let cluster_name = self
            .session
            .read(cx)
            .active_cluster()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "No cluster".to_owned());
        let collapsed = self.sidebar_collapsed;

        Sidebar::new("sidebar")
            .collapsed(collapsed)
            .header(
                SidebarHeader::new().child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(
                            div()
                                .flex_shrink_0()
                                .size(px(28.))
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().primary)
                                .text_color(cx.theme().primary_foreground)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(gpui_kit::component::Icon::new(IconName::Ship).small()),
                        )
                        .when_not_collapsed(collapsed, |row| {
                            row.child(
                                v_flex()
                                    .min_w_0()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Kubiru"))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(cluster_name),
                                    ),
                            )
                        }),
                ),
            )
            .children(groups)
    }

    fn render_top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let session = self.session.read(cx);
        let namespaced = match self.route {
            Route::Overview => true,
            Route::Resources(kind) => kind.namespaced(),
            _ => false,
        };
        let connected = session.client().is_some();
        let namespaces_failed = matches!(session.namespaces(), Load::Failed(_));
        let collapsed = self.sidebar_collapsed;

        h_flex()
            .h(px(52.))
            .flex_shrink_0()
            .px_4()
            .gap_3()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("toggle-sidebar")
                    .ghost()
                    .small()
                    .icon(if collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .tooltip(crate::ui::with_shortcut(
                        if collapsed {
                            "Show sidebar"
                        } else {
                            "Hide sidebar"
                        },
                        "B",
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_sidebar_collapsed(!collapsed, cx)
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .when_visible(namespaced && connected, |row| {
                        row.child(
                            div().w(px(240.)).child(
                                Select::new(&self.namespace)
                                    .small()
                                    .icon(IconName::Folder)
                                    .placeholder("All namespaces")
                                    .search_placeholder("Find a namespace")
                                    .accessibility_label("Namespace")
                                    .disabled(namespaces_failed),
                            ),
                        )
                    }),
            )
    }

    fn page(&self) -> AnyElement {
        match self.route {
            Route::Overview => self.overview.clone().into_any_element(),
            Route::Resources(_) => self.resources.clone().into_any_element(),
            Route::Clusters => self.clusters.clone().into_any_element(),
            Route::Settings => self.settings.clone().into_any_element(),
            Route::Manifest => match &self.manifest {
                Some(m) => m.clone().into_any_element(),
                None => self.overview.clone().into_any_element(),
            },
        }
    }
}

/// Builder helpers that read better than nested `when`.
trait When: Sized {
    fn when_not_collapsed(self, collapsed: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if collapsed { self } else { f(self) }
    }
    fn when_visible(self, visible: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if visible { f(self) } else { self }
    }
}

impl<T: ParentElement> When for T {}

fn namespace_items(session: &Entity<Session>, cx: &App) -> Vec<NamespaceItem> {
    let session = session.read(cx);
    let mut items = vec![NamespaceItem {
        title: "All namespaces".into(),
        value: None,
    }];
    let mut names: Vec<String> = session.namespaces().ready().cloned().unwrap_or_default();
    // Keep a remembered namespace selectable even if it could not be listed.
    if let Some(current) = session.namespace()
        && !names.iter().any(|n| n == current)
    {
        names.push(current.to_owned());
    }
    items.extend(names.into_iter().map(|n| NamespaceItem {
        title: n.clone().into(),
        value: Some(n),
    }));
    items
}

fn kind_icon(kind: ResourceKind) -> IconName {
    match kind {
        ResourceKind::Pods => IconName::Box,
        ResourceKind::Deployments => IconName::Rocket,
        ResourceKind::StatefulSets => IconName::Database,
        ResourceKind::DaemonSets => IconName::Layers,
        ResourceKind::Jobs => IconName::Play,
        ResourceKind::CronJobs => IconName::CalendarClock,
        ResourceKind::Services => IconName::Network,
        ResourceKind::Ingresses => IconName::Globe,
        ResourceKind::ConfigMaps => IconName::FileText,
        ResourceKind::PersistentVolumeClaims => IconName::HardDrive,
        ResourceKind::PersistentVolumes => IconName::Database,
        ResourceKind::Nodes => IconName::Server,
        ResourceKind::Namespaces => IconName::Folder,
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("workspace")
            .key_context(WORKSPACE)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                if let Route::Resources(_) = this.route {
                    let handle = this.resources.read(cx).filter_focus_handle(cx);
                    handle.focus(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                let collapsed = !this.sidebar_collapsed;
                this.set_sidebar_collapsed(collapsed, cx);
            }))
            .on_action(
                cx.listener(|this, _: &OpenSettings, w, cx| this.navigate(Route::Settings, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenClusters, w, cx| this.navigate(Route::Clusters, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &AddCluster, w, cx| this.open_cluster_form(None, w, cx)),
            )
            .on_action(
                cx.listener(|this, _: &GoOverview, w, cx| this.navigate(Route::Overview, w, cx)),
            )
            .on_action(cx.listener(|this, _: &GoPods, w, cx| {
                this.navigate(Route::Resources(ResourceKind::Pods), w, cx)
            }))
            .on_action(cx.listener(|this, _: &GoDeployments, w, cx| {
                this.navigate(Route::Resources(ResourceKind::Deployments), w, cx)
            }))
            .on_action(cx.listener(|this, _: &GoServices, w, cx| {
                this.navigate(Route::Resources(ResourceKind::Services), w, cx)
            }))
            .on_action(cx.listener(|this, _: &GoNodes, w, cx| {
                this.navigate(Route::Resources(ResourceKind::Nodes), w, cx)
            }))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_top_bar(cx))
                    .child(div().flex_1().min_h_0().child(self.page())),
            )
    }
}
