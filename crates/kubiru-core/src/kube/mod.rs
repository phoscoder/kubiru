//! Talking to the Kubernetes API and presenting what it returns.

mod client;
pub mod manifest;
pub mod overview;
mod resource;
pub mod table;

pub use client::{KubeClient, KubeError, ServerVersion};
pub use resource::{Category, ObjectRef, ResourceKind, is_valid_name, is_valid_namespace};
