//! Domain logic for Kubiru, kept free of any UI code so it can be tested on
//! its own.
//!
//! - [`cluster`] — connection settings and their validation
//! - [`store`] — SQLite persistence for clusters and preferences
//! - [`vault`] — bearer tokens in the operating system's keychain
//! - [`registry`] — the two kept in step
//! - [`kube`] — the Kubernetes API client and presentation helpers

pub mod cluster;
pub mod kube;
pub mod registry;
pub mod secret;
pub mod store;
pub mod vault;

pub use cluster::{Cluster, ClusterDraft, ClusterId, TlsVerification};
pub use registry::{Credentials, Registry, RegistryError};
pub use secret::Secret;
