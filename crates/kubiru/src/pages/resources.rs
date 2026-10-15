//! A sortable, filterable table of one resource kind.

use std::cmp::Ordering;

use chrono::Utc;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{
    Column, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use kubiru_core::kube::table::{self, ColumnKind, ColumnSpec, Row};
use kubiru_core::kube::{ObjectRef, ResourceKind};

use crate::actions::{CopySelectedName, DeleteSelected, OpenSelected};
use crate::pages::{NavEvent, confirm_delete, delete_object};
use crate::session::{Load, Session, SessionEvent, client_and_namespace};
use crate::ui;

/// Rows of one kind, filtered and sorted for display.
pub struct ResourceTable {
    kind: ResourceKind,
    columns: Vec<ColumnSpec>,
    rows: Vec<Row>,
    /// Indexes into `rows`, in display order.
    visible: Vec<usize>,
    filter: String,
    sort: Option<(usize, ColumnSort)>,
}

impl ResourceTable {
    pub fn new(kind: ResourceKind) -> Self {
        Self {
            kind,
            columns: table::columns(kind),
            rows: Vec::new(),
            visible: Vec::new(),
            filter: String::new(),
            sort: None,
        }
    }

    pub fn set_rows(&mut self, kind: ResourceKind, rows: Vec<Row>) {
        if kind != self.kind {
            self.kind = kind;
            self.columns = table::columns(kind);
            self.sort = None;
        }
        self.rows = rows;
        self.update_visible();
    }

    pub fn set_filter(&mut self, filter: &str) {
        self.filter = filter.trim().to_lowercase();
        self.update_visible();
    }

    /// The row shown at `index`.
    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(*self.visible.get(index)?)
    }

    pub fn total(&self) -> usize {
        self.rows.len()
    }

    pub fn shown(&self) -> usize {
        self.visible.len()
    }

    fn update_visible(&mut self) {
        let filter = self.filter.as_str();
        self.visible = (0..self.rows.len())
            .filter(|&i| self.rows[i].matches(filter))
            .collect();
        if let Some((col, sort)) = self.sort {
            let rows = &self.rows;
            self.visible.sort_by(|&a, &b| {
                let order = match (rows[a].cells.get(col), rows[b].cells.get(col)) {
                    (Some(x), Some(y)) => x.sort.compare(&y.sort),
                    _ => Ordering::Equal,
                };
                // Keep the order stable on ties by falling back to the name.
                let order =
                    order.then_with(|| rows[a].cells[0].sort.compare(&rows[b].cells[0].sort));
                if matches!(sort, ColumnSort::Descending) {
                    order.reverse()
                } else {
                    order
                }
            });
        }
    }
}

impl TableDelegate for ResourceTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.visible.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let spec = self.columns[col_ix];
        let column = Column::new(
            SharedString::from(spec.title),
            SharedString::from(spec.title),
        )
        .width(px(spec.width))
        .sortable();
        match spec.kind {
            ColumnKind::Number => column.text_right(),
            _ => column,
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.sort = match sort {
            ColumnSort::Default => None,
            sort => Some((col_ix, sort)),
        };
        self.update_visible();
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(row) = self.row(row_ix) else {
            return div().into_any_element();
        };
        let text = row
            .cells
            .get(col_ix)
            .map(|c| c.text.clone())
            .unwrap_or_default();
        match self.columns[col_ix].kind {
            ColumnKind::Status => ui::status(text, row.tone, cx).into_any_element(),
            _ if col_ix == 0 => div()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .child(text)
                .into_any_element(),
            ColumnKind::Number => div().w_full().text_right().child(text).into_any_element(),
            _ => div().truncate().child(text).into_any_element(),
        }
    }

    fn context_menu(
        &mut self,
        _: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> PopupMenu {
        menu.menu_with_icon("Open manifest", IconName::FileCode, Box::new(OpenSelected))
            .menu_with_icon("Copy name", IconName::Copy, Box::new(CopySelectedName))
            .separator()
            .menu_with_icon("Delete\u{2026}", IconName::Trash, Box::new(DeleteSelected))
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let message = if self.rows.is_empty() {
            format!("No {} found.", self.kind.title().to_lowercase())
        } else {
            "No rows match the filter.".to_owned()
        };
        h_flex()
            .size_full()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message)
    }
}

pub struct ResourcesPage {
    session: Entity<Session>,
    kind: ResourceKind,
    table: Entity<TableState<ResourceTable>>,
    filter: Entity<InputState>,
    load: Load<()>,
    loaded_at: Option<String>,
    /// Identifies the latest request so older responses are dropped.
    request: u64,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NavEvent> for ResourcesPage {}

impl ResourcesPage {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let kind = ResourceKind::Pods;
        let table = cx.new(|cx| {
            TableState::new(ResourceTable::new(kind), window, cx)
                .col_movable(false)
                .sortable(true)
        });
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter by any column"));
        let subscriptions = vec![
            cx.subscribe(&session, |this, _, event, cx| {
                if *event == SessionEvent::ContextChanged {
                    this.reload(cx);
                }
            }),
            cx.subscribe(&filter, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value();
                    this.table.update(cx, |t, cx| {
                        t.delegate_mut().set_filter(&text);
                        t.refresh(cx);
                    });
                    cx.notify();
                }
            }),
            cx.subscribe(&table, |this, _, event: &TableEvent, cx| {
                if let TableEvent::DoubleClickedRow(ix) = event {
                    this.open_row(*ix, cx);
                }
            }),
        ];
        let mut this = Self {
            session,
            kind,
            table,
            filter,
            load: Load::Idle,
            loaded_at: None,
            request: 0,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.reload(cx);
        this
    }

    /// Show `kind`, loading it if it is not already shown.
    pub fn show(&mut self, kind: ResourceKind, cx: &mut Context<Self>) {
        if kind != self.kind {
            self.kind = kind;
            self.table.update(cx, |t, cx| {
                t.delegate_mut().set_rows(kind, Vec::new());
                t.clear_selection(cx);
                t.refresh(cx);
            });
            self.load = Load::Idle;
            self.loaded_at = None;
            self.reload(cx);
        }
    }

    /// Focus for keyboard navigation of the rows.
    pub fn table_focus_handle(&self, cx: &App) -> FocusHandle {
        self.table.read(cx).focus_handle(cx)
    }

    pub fn filter_focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (client, namespace, _) = client_and_namespace(&self.session, cx);
        self.request += 1;
        let Some(client) = client else {
            self.load = Load::Idle;
            self.table.update(cx, |t, cx| {
                t.delegate_mut().set_rows(self.kind, Vec::new());
                t.refresh(cx);
            });
            cx.notify();
            return;
        };
        let (kind, request) = (self.kind, self.request);
        self.load = Load::Loading;
        cx.notify();
        let task = cx.background_spawn(async move {
            client
                .list(kind, namespace.as_deref())
                .map(|items| table::rows(kind, &items, Utc::now()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.request != request {
                    return;
                }
                match result {
                    Ok(rows) => {
                        this.table.update(cx, |t, cx| {
                            t.delegate_mut().set_rows(kind, rows);
                            t.refresh(cx);
                        });
                        this.load = Load::Ready(());
                        this.loaded_at = Some(chrono::Local::now().format("%H:%M:%S").to_string());
                    }
                    Err(e) => this.load = Load::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn selected(&self, cx: &App) -> Option<ObjectRef> {
        let table = self.table.read(cx);
        let ix = table.right_clicked_row().or(table.selected_row())?;
        table.delegate().row(ix).map(|r| r.object.clone())
    }

    fn open_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.table.read(cx).delegate().row(ix) {
            cx.emit(NavEvent::Open(row.object.clone()));
        }
    }

    fn on_open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(object) = self.selected(cx) {
            cx.emit(NavEvent::Open(object));
        }
    }

    fn on_copy_name(&mut self, _: &CopySelectedName, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(object) = self.selected(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(object.name.clone()));
            window.push_notification(format!("Copied \u{201c}{}\u{201d}", object.name), cx);
        }
    }

    fn on_delete_selected(
        &mut self,
        _: &DeleteSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(object) = self.selected(cx) else {
            return;
        };
        let session = self.session.clone();
        let page = cx.entity().downgrade();
        confirm_delete(&object, window, cx, move |object, window, cx| {
            let page = page.clone();
            delete_object(&session, object, window, cx, move |_, cx| {
                page.update(cx, |page, cx| page.reload(cx)).ok();
            });
        });
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match self.render_placeholder(cx) {
            // The placeholder takes the table's focus so keyboard focus is
            // never left pointing at something that is not on screen.
            Some(placeholder) => div()
                .size_full()
                .track_focus(&self.table.read(cx).focus_handle(cx))
                .child(placeholder)
                .into_any_element(),
            None => DataTable::new(&self.table).stripe(true).into_any_element(),
        }
    }

    /// What to show instead of the table, if anything.
    fn render_placeholder(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let session = self.session.read(cx);
        if session.active_cluster().is_none() {
            let add = Button::new("add-first-cluster")
                .primary()
                .icon(IconName::Plus)
                .label("Add a cluster")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(NavEvent::AddCluster)));
            return Some(
                ui::empty_state(
                    IconName::Ship,
                    "No cluster yet",
                    "Add a cluster to browse its workloads. You need the API server address and a bearer token.",
                    Some(add.into_any_element()),
                    cx,
                )
                .into_any_element(),
            );
        }
        match session.connection() {
            Load::Loading | Load::Idle => {
                return Some(
                    ui::empty_state(
                        IconName::Plug,
                        "Connecting\u{2026}",
                        "Reading the token from the system keychain and contacting the cluster.",
                        None,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            Load::Failed(error) => {
                let retry = Button::new("reconnect")
                    .outline()
                    .icon(IconName::RefreshCw)
                    .label("Try again")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |s, cx| s.reconnect(cx));
                    }));
                return Some(
                    ui::empty_state(
                        IconName::Unplug,
                        "Could not connect",
                        error.clone(),
                        Some(retry.into_any_element()),
                        cx,
                    )
                    .into_any_element(),
                );
            }
            Load::Ready(_) => {}
        }
        if let Load::Failed(error) = &self.load {
            let retry = Button::new("retry")
                .outline()
                .icon(IconName::RefreshCw)
                .label("Try again")
                .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));
            return Some(
                ui::empty_state(
                    IconName::TriangleAlert,
                    format!("Could not load {}", self.kind.title().to_lowercase()),
                    error.clone(),
                    Some(retry.into_any_element()),
                    cx,
                )
                .into_any_element(),
            );
        }
        None
    }
}

impl Focusable for ResourcesPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ResourcesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (shown, total) = {
            let t = self.table.read(cx).delegate();
            (t.shown(), t.total())
        };
        let count = if shown == total {
            format!("{total}")
        } else {
            format!("{shown} of {total}")
        };
        let scope = if self.kind.namespaced() {
            match self.session.read(cx).namespace() {
                Some(ns) => format!("in {ns}"),
                None => "in all namespaces".to_owned(),
            }
        } else {
            "cluster-wide".to_owned()
        };
        let subtitle = match (&self.load, &self.loaded_at) {
            (Load::Loading, _) => format!("{count} \u{b7} {scope} \u{b7} refreshing\u{2026}"),
            (_, Some(at)) => format!("{count} \u{b7} {scope} \u{b7} updated {at}"),
            _ => scope,
        };
        let refresh = Button::new("refresh")
            .ghost()
            .icon(IconName::RefreshCw)
            .loading(self.load.is_loading())
            .tooltip(ui::with_shortcut("Refresh", "R"))
            .on_click(cx.listener(|this, _, _, cx| this.reload(cx)));

        v_flex()
            .id("resources-page")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_copy_name))
            .on_action(cx.listener(Self::on_delete_selected))
            .size_full()
            .gap_4()
            .p_6()
            .child(ui::page_header(
                self.kind.title(),
                Some(subtitle.into()),
                h_flex()
                    .gap_2()
                    .child(
                        div().w(px(260.)).child(
                            Input::new(&self.filter)
                                .small()
                                .cleanable(true)
                                .prefix(gpui_kit::component::Icon::new(IconName::Search).small()),
                        ),
                    )
                    .child(refresh),
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when(
                        self.load.is_loading() && self.table.read(cx).delegate().total() == 0,
                        |d| d.opacity(0.6),
                    )
                    .child(self.render_body(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pods() -> Vec<Row> {
        let now = Utc::now();
        ["web-10", "web-2", "api-1"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let item = json!({
                    "metadata": {"name": name, "namespace": if i == 2 { "backend" } else { "frontend" }},
                    "spec": {"containers": [{}]},
                    "status": {"phase": "Running", "containerStatuses": [
                        {"ready": true, "restartCount": i, "state": {"running": {}}}
                    ]}
                });
                table::row(ResourceKind::Pods, &item, now).unwrap()
            })
            .collect()
    }

    fn names(t: &ResourceTable) -> Vec<String> {
        (0..t.shown())
            .map(|i| t.row(i).unwrap().object.name.clone())
            .collect()
    }

    #[test]
    fn filtering_matches_any_column() {
        let mut t = ResourceTable::new(ResourceKind::Pods);
        t.set_rows(ResourceKind::Pods, pods());
        t.set_filter(" Backend ");
        assert_eq!(names(&t), ["api-1"]);
        t.set_filter("");
        assert_eq!(t.shown(), 3);
    }

    #[test]
    fn sorting_is_natural_and_reversible() {
        let mut t = ResourceTable::new(ResourceKind::Pods);
        t.set_rows(ResourceKind::Pods, pods());
        t.sort = Some((0, ColumnSort::Ascending));
        t.update_visible();
        assert_eq!(names(&t), ["api-1", "web-2", "web-10"]);
        // Restarts (column 4) descending.
        t.sort = Some((4, ColumnSort::Descending));
        t.update_visible();
        assert_eq!(names(&t), ["api-1", "web-2", "web-10"]);
        t.sort = Some((4, ColumnSort::Ascending));
        t.update_visible();
        assert_eq!(names(&t), ["web-10", "web-2", "api-1"]);
    }

    #[test]
    fn switching_kind_resets_columns_and_sort() {
        let mut t = ResourceTable::new(ResourceKind::Pods);
        t.sort = Some((3, ColumnSort::Ascending));
        t.set_rows(ResourceKind::Nodes, Vec::new());
        assert!(t.sort.is_none());
        assert_eq!(t.columns, table::columns(ResourceKind::Nodes));
    }
}
