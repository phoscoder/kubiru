//! The resource types Kubiru can browse, and where they live in the API.

use std::fmt;

/// A browsable resource type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Pods,
    Deployments,
    StatefulSets,
    DaemonSets,
    Jobs,
    CronJobs,
    Services,
    Ingresses,
    ConfigMaps,
    PersistentVolumeClaims,
    Nodes,
    Namespaces,
    PersistentVolumes,
}

/// Groups used to organise the navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Workloads,
    Network,
    Config,
    Storage,
    Cluster,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Workloads,
        Category::Network,
        Category::Config,
        Category::Storage,
        Category::Cluster,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Workloads => "Workloads",
            Category::Network => "Network",
            Category::Config => "Configuration",
            Category::Storage => "Storage",
            Category::Cluster => "Cluster",
        }
    }

    pub fn kinds(self) -> impl Iterator<Item = ResourceKind> {
        ResourceKind::ALL
            .into_iter()
            .filter(move |k| k.category() == self)
    }
}

impl ResourceKind {
    pub const ALL: [ResourceKind; 13] = [
        ResourceKind::Pods,
        ResourceKind::Deployments,
        ResourceKind::StatefulSets,
        ResourceKind::DaemonSets,
        ResourceKind::Jobs,
        ResourceKind::CronJobs,
        ResourceKind::Services,
        ResourceKind::Ingresses,
        ResourceKind::ConfigMaps,
        ResourceKind::PersistentVolumeClaims,
        ResourceKind::PersistentVolumes,
        ResourceKind::Nodes,
        ResourceKind::Namespaces,
    ];

    /// Path prefix of the API group/version that serves this kind.
    pub fn api_prefix(self) -> &'static str {
        match self {
            Self::Deployments | Self::StatefulSets | Self::DaemonSets => "apis/apps/v1",
            Self::Jobs | Self::CronJobs => "apis/batch/v1",
            Self::Ingresses => "apis/networking.k8s.io/v1",
            Self::Pods
            | Self::Services
            | Self::ConfigMaps
            | Self::PersistentVolumeClaims
            | Self::Nodes
            | Self::Namespaces
            | Self::PersistentVolumes => "api/v1",
        }
    }

    /// The lower-case plural used in URLs.
    pub fn plural(self) -> &'static str {
        match self {
            Self::Pods => "pods",
            Self::Deployments => "deployments",
            Self::StatefulSets => "statefulsets",
            Self::DaemonSets => "daemonsets",
            Self::Jobs => "jobs",
            Self::CronJobs => "cronjobs",
            Self::Services => "services",
            Self::Ingresses => "ingresses",
            Self::ConfigMaps => "configmaps",
            Self::PersistentVolumeClaims => "persistentvolumeclaims",
            Self::Nodes => "nodes",
            Self::Namespaces => "namespaces",
            Self::PersistentVolumes => "persistentvolumes",
        }
    }

    /// The `kind` field of objects of this type.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Pods => "Pod",
            Self::Deployments => "Deployment",
            Self::StatefulSets => "StatefulSet",
            Self::DaemonSets => "DaemonSet",
            Self::Jobs => "Job",
            Self::CronJobs => "CronJob",
            Self::Services => "Service",
            Self::Ingresses => "Ingress",
            Self::ConfigMaps => "ConfigMap",
            Self::PersistentVolumeClaims => "PersistentVolumeClaim",
            Self::Nodes => "Node",
            Self::Namespaces => "Namespace",
            Self::PersistentVolumes => "PersistentVolume",
        }
    }

    /// Human-readable plural, for navigation and headings.
    pub fn title(self) -> &'static str {
        match self {
            Self::Pods => "Pods",
            Self::Deployments => "Deployments",
            Self::StatefulSets => "Stateful Sets",
            Self::DaemonSets => "Daemon Sets",
            Self::Jobs => "Jobs",
            Self::CronJobs => "Cron Jobs",
            Self::Services => "Services",
            Self::Ingresses => "Ingresses",
            Self::ConfigMaps => "Config Maps",
            Self::PersistentVolumeClaims => "Volume Claims",
            Self::Nodes => "Nodes",
            Self::Namespaces => "Namespaces",
            Self::PersistentVolumes => "Persistent Volumes",
        }
    }

    /// Whether objects belong to a namespace.
    pub fn namespaced(self) -> bool {
        !matches!(
            self,
            Self::Nodes | Self::Namespaces | Self::PersistentVolumes
        )
    }

    pub fn category(self) -> Category {
        match self {
            Self::Pods
            | Self::Deployments
            | Self::StatefulSets
            | Self::DaemonSets
            | Self::Jobs
            | Self::CronJobs => Category::Workloads,
            Self::Services | Self::Ingresses => Category::Network,
            Self::ConfigMaps => Category::Config,
            Self::PersistentVolumeClaims | Self::PersistentVolumes => Category::Storage,
            Self::Nodes | Self::Namespaces => Category::Cluster,
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

/// Identifies a single object.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub kind: ResourceKind,
    /// `None` for cluster-scoped kinds.
    pub namespace: Option<String>,
    pub name: String,
}

impl fmt::Display for ObjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.namespace {
            Some(ns) => write!(f, "{}/{ns}/{}", self.kind.plural(), self.name),
            None => write!(f, "{}/{}", self.kind.plural(), self.name),
        }
    }
}

/// Longest object name the API accepts.
const NAME_MAX: usize = 253;

/// Whether `name` is a valid object name (a DNS-1123 subdomain). Names are
/// checked before being put in a URL so a crafted value can never change the
/// request path.
pub fn is_valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= NAME_MAX
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'.')
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
}

/// Whether `name` is a valid namespace name (a DNS-1123 label).
pub fn is_valid_namespace(name: &str) -> bool {
    name.len() <= 63 && !name.contains('.') && is_valid_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_in_exactly_one_category() {
        let total: usize = Category::ALL.iter().map(|c| c.kinds().count()).sum();
        assert_eq!(total, ResourceKind::ALL.len());
    }

    #[test]
    fn cluster_scoped_kinds() {
        let scoped: Vec<_> = ResourceKind::ALL
            .into_iter()
            .filter(|k| !k.namespaced())
            .collect();
        assert_eq!(
            scoped,
            [
                ResourceKind::PersistentVolumes,
                ResourceKind::Nodes,
                ResourceKind::Namespaces
            ]
        );
    }

    #[test]
    fn names_follow_dns_1123() {
        assert!(is_valid_name("nginx-7c5ddbdf54-x2x9q"));
        assert!(is_valid_name("ip-10-0-0-1.ec2.internal"));
        for bad in [
            "",
            "-a",
            "a-",
            "A",
            "a/b",
            "..",
            "a b",
            "a%2Fb",
            &"a".repeat(254),
        ] {
            assert!(!is_valid_name(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn namespaces_are_labels() {
        assert!(is_valid_namespace("kube-system"));
        assert!(!is_valid_namespace("a.b"));
        assert!(!is_valid_namespace(&"a".repeat(64)));
    }
}
