//! What the window is connected to: the saved clusters, the active one, its
//! API client and the chosen namespace.
//!
//! Keychain access and network calls run on background threads. Each change
//! of cluster or namespace bumps [`Session::epoch`], and pages drop results
//! that were requested under an older epoch.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Context, EventEmitter};
use kubiru_core::kube::{KubeClient, KubeError};
use kubiru_core::{Cluster, ClusterId, Registry};

/// Progress of something loaded in the background.
#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self, Load::Loading)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionEvent {
    /// Clusters were added, edited or removed.
    ClustersChanged,
    /// The active cluster, its connection or the namespace changed.
    ContextChanged,
}

pub struct Session {
    registry: Arc<Registry>,
    clusters: Vec<Cluster>,
    clusters_error: Option<String>,
    active: Option<ClusterId>,
    client: Load<KubeClient>,
    namespaces: Load<Vec<String>>,
    namespace: Option<String>,
    epoch: u64,
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn new(registry: Arc<Registry>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            registry,
            clusters: Vec::new(),
            clusters_error: None,
            active: None,
            client: Load::Idle,
            namespaces: Load::Idle,
            namespace: None,
            epoch: 0,
        };
        this.load_clusters();
        let remembered = this
            .registry
            .with_store(|s| s.active_cluster())
            .ok()
            .flatten();
        let initial = remembered
            .filter(|id| this.clusters.iter().any(|c| c.id == *id))
            .or_else(|| this.clusters.first().map(|c| c.id));
        this.activate(initial, cx);
        this
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }

    pub fn clusters(&self) -> &[Cluster] {
        &self.clusters
    }

    /// Why the cluster list could not be read, if it could not.
    pub fn clusters_error(&self) -> Option<&str> {
        self.clusters_error.as_deref()
    }

    pub fn active_cluster(&self) -> Option<&Cluster> {
        let id = self.active?;
        self.clusters.iter().find(|c| c.id == id)
    }

    pub fn connection(&self) -> &Load<KubeClient> {
        &self.client
    }

    pub fn client(&self) -> Option<KubeClient> {
        self.client.ready().cloned()
    }

    pub fn namespaces(&self) -> &Load<Vec<String>> {
        &self.namespaces
    }

    /// The selected namespace; `None` means all namespaces.
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Make `id` the active cluster and connect to it.
    pub fn select_cluster(&mut self, id: ClusterId, cx: &mut Context<Self>) {
        if self.active != Some(id) {
            self.activate(Some(id), cx);
        }
    }

    pub fn select_namespace(&mut self, namespace: Option<String>, cx: &mut Context<Self>) {
        if self.namespace == namespace {
            return;
        }
        if let Some(id) = self.active {
            let ns = namespace.clone();
            self.registry
                .with_store(|s| s.set_namespace(id, ns.as_deref()))
                .ok();
        }
        self.namespace = namespace;
        self.epoch += 1;
        cx.emit(SessionEvent::ContextChanged);
        cx.notify();
    }

    /// Connect again, e.g. after a network failure.
    pub fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.activate(self.active, cx);
    }

    /// Re-read the cluster list after it was changed. `select` becomes the
    /// active cluster; otherwise the current one is kept if it still exists.
    pub fn reload_clusters(&mut self, select: Option<ClusterId>, cx: &mut Context<Self>) {
        self.load_clusters();
        cx.emit(SessionEvent::ClustersChanged);
        let still_there = |id: &ClusterId| self.clusters.iter().any(|c| c.id == *id);
        let next = select
            .filter(still_there)
            .or(self.active.filter(still_there))
            .or_else(|| self.clusters.first().map(|c| c.id));
        // Settings of the active cluster may have changed, so always reconnect.
        self.activate(next, cx);
    }

    fn load_clusters(&mut self) {
        match self.registry.clusters() {
            Ok(clusters) => {
                self.clusters = clusters;
                self.clusters_error = None;
            }
            Err(e) => {
                self.clusters.clear();
                self.clusters_error = Some(e.to_string());
            }
        }
    }

    fn activate(&mut self, id: Option<ClusterId>, cx: &mut Context<Self>) {
        self.active = id;
        self.epoch += 1;
        self.registry.with_store(|s| s.set_active_cluster(id)).ok();
        self.namespace =
            id.and_then(|id| self.registry.with_store(|s| s.namespace(id)).ok().flatten());
        let Some(id) = id else {
            self.client = Load::Idle;
            self.namespaces = Load::Idle;
            cx.emit(SessionEvent::ContextChanged);
            cx.notify();
            return;
        };
        self.client = Load::Loading;
        self.namespaces = Load::Loading;
        cx.emit(SessionEvent::ContextChanged);
        cx.notify();

        let epoch = self.epoch;
        let registry = self.registry.clone();
        let task = cx.background_spawn(async move {
            let mut client = registry
                .credentials(id)
                .map_err(|e| e.to_string())
                .and_then(|creds| KubeClient::new(&creds).map_err(|e| e.to_string()));
            let namespaces = client.as_ref().ok().map(KubeClient::namespaces);
            // Listing namespaces doubles as the connection check. A server that
            // cannot be reached, or that rejects the token, is not connected; a
            // token that may simply not list namespaces (RBAC) still is.
            if let Some(Err(e)) = &namespaces
                && (e.is_transient() || matches!(e, KubeError::Unauthorized))
            {
                client = Err(e.to_string());
            }
            let namespaces = namespaces.map(|r| r.map_err(|e| e.to_string()));
            (client, namespaces)
        });
        cx.spawn(async move |this, cx| {
            let (client, namespaces) = task.await;
            this.update(cx, |this, cx| {
                if this.epoch != epoch {
                    return;
                }
                this.client = match client {
                    Ok(client) => Load::Ready(client),
                    Err(e) => Load::Failed(e),
                };
                this.namespaces = match namespaces {
                    Some(Ok(names)) => Load::Ready(names),
                    Some(Err(e)) => Load::Failed(e),
                    None => Load::Idle,
                };
                // A connection is a new context for every page.
                this.epoch += 1;
                cx.emit(SessionEvent::ContextChanged);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// Convenience for reading the session from a page.
pub fn client_and_namespace(
    session: &gpui_kit::Entity<Session>,
    cx: &App,
) -> (Option<KubeClient>, Option<String>, u64) {
    let s = session.read(cx);
    (s.client(), s.namespace().map(str::to_owned), s.epoch())
}
