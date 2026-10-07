//! Talking to the Kubernetes API and presenting what it returns.

mod resource;

pub use resource::{Category, ObjectRef, ResourceKind, is_valid_name, is_valid_namespace};
