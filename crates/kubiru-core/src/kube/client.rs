//! A small, blocking client for the parts of the Kubernetes API Kubiru uses.
//!
//! The client is deliberately strict:
//! - TLS is verified unless the user explicitly chose otherwise for the cluster;
//! - redirects are not followed, so the token is only ever sent to the
//!   configured server;
//! - every name that goes into a URL is validated first;
//! - requests time out and response bodies are size-limited.
//!
//! Calls block, so run them on a background thread.

use std::io::Read;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use super::resource::{ObjectRef, ResourceKind, is_valid_name, is_valid_namespace};
use crate::cluster::TlsVerification;
use crate::registry::Credentials;
use crate::secret::Secret;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Large clusters can return big lists, but nothing near this.
const MAX_BODY: u64 = 64 * 1024 * 1024;
const USER_AGENT: &str = concat!("kubiru/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, thiserror::Error)]
pub enum KubeError {
    #[error("\u{201c}{0}\u{201d} is not a valid Kubernetes name")]
    InvalidName(String),
    #[error("the CA certificate could not be read: {0}")]
    InvalidCertificate(String),
    #[error("could not reach the API server: {0}")]
    Connect(String),
    #[error("the API server did not respond in time")]
    Timeout,
    #[error("the cluster rejected the token; it may have expired or been revoked")]
    Unauthorized,
    #[error("permission denied: {0}")]
    Forbidden(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("the object changed on the server since it was loaded; reload it and try again")]
    Conflict(String),
    #[error("the API server rejected the request: {0}")]
    Invalid(String),
    #[error("the API server returned an error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("unexpected response from the API server: {0}")]
    Decode(String),
}

impl KubeError {
    /// Whether retrying the same request might succeed.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Connect(_) | Self::Timeout)
            || matches!(self, Self::Api { status, .. } if *status >= 500)
    }
}

pub type Result<T, E = KubeError> = std::result::Result<T, E>;

/// Version information reported by `/version`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerVersion {
    #[serde(default)]
    pub git_version: String,
    #[serde(default)]
    pub platform: String,
}

#[derive(Clone)]
pub struct KubeClient {
    http: Client,
    base: Url,
    token: Secret,
}

impl std::fmt::Debug for KubeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KubeClient")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl KubeClient {
    pub fn new(credentials: &Credentials) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(credentials.cluster.api_url.scheme() == "https")
            .use_rustls_tls();
        builder = match &credentials.cluster.tls {
            TlsVerification::System => builder,
            TlsVerification::CustomCa(pem) => {
                let certs = reqwest::Certificate::from_pem_bundle(pem.as_bytes())
                    .map_err(|e| KubeError::InvalidCertificate(e.to_string()))?;
                if certs.is_empty() {
                    return Err(KubeError::InvalidCertificate(
                        "no certificates found".into(),
                    ));
                }
                certs
                    .into_iter()
                    .fold(builder.tls_built_in_root_certs(false), |b, c| {
                        b.add_root_certificate(c)
                    })
            }
            TlsVerification::Insecure => builder.danger_accept_invalid_certs(true),
        };
        let http = builder
            .build()
            .map_err(|e| KubeError::Connect(e.to_string()))?;
        Ok(Self {
            http,
            base: credentials.cluster.api_url.clone(),
            token: credentials.token.clone(),
        })
    }

    /// Check the address and token by asking the server for its version and
    /// listing namespaces (which requires a valid token).
    pub fn verify(&self) -> Result<ServerVersion> {
        let version = self.get_json(self.endpoint(&["version"]))?;
        let version =
            serde_json::from_value(version).map_err(|e| KubeError::Decode(e.to_string()))?;
        self.list(ResourceKind::Namespaces, None).map(|_| version)
    }

    /// List objects of `kind`, in `namespace` or across all namespaces.
    pub fn list(&self, kind: ResourceKind, namespace: Option<&str>) -> Result<Vec<Value>> {
        let url = self.collection_url(kind, namespace)?;
        let mut body = self.get_json(url)?;
        match body.get_mut("items").map(Value::take) {
            Some(Value::Array(items)) => Ok(items),
            Some(Value::Null) | None => Ok(Vec::new()),
            Some(_) => Err(KubeError::Decode("`items` is not a list".into())),
        }
    }

    /// Names of all namespaces, sorted.
    pub fn namespaces(&self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .list(ResourceKind::Namespaces, None)?
            .iter()
            .filter_map(|ns| ns.pointer("/metadata/name")?.as_str().map(str::to_owned))
            .collect();
        names.sort();
        Ok(names)
    }

    pub fn get(&self, object: &ObjectRef) -> Result<Value> {
        self.get_json(self.object_url(object)?)
    }

    /// Replace an object with `manifest` (an HTTP PUT). The manifest's
    /// `resourceVersion` makes this fail with [`KubeError::Conflict`] if the
    /// object changed in the meantime.
    pub fn replace(&self, object: &ObjectRef, manifest: &Value) -> Result<Value> {
        let body = serde_json::to_vec(manifest).map_err(|e| KubeError::Decode(e.to_string()))?;
        let request = self
            .authorized(self.http.put(self.object_url(object)?))
            .header(CONTENT_TYPE, "application/json")
            .body(body);
        read_json(self.send(request)?)
    }

    /// Delete an object. Dependents are removed in the background, as with
    /// `kubectl delete`.
    pub fn delete(&self, object: &ObjectRef) -> Result<()> {
        let body =
            br#"{"kind":"DeleteOptions","apiVersion":"v1","propagationPolicy":"Background"}"#;
        let request = self
            .authorized(self.http.delete(self.object_url(object)?))
            .header(CONTENT_TYPE, "application/json")
            .body(body.as_slice());
        self.send(request).map(drop)
    }

    fn collection_url(&self, kind: ResourceKind, namespace: Option<&str>) -> Result<Url> {
        let mut segments: Vec<&str> = kind.api_prefix().split('/').collect();
        if let (true, Some(ns)) = (kind.namespaced(), namespace) {
            if !is_valid_namespace(ns) {
                return Err(KubeError::InvalidName(ns.to_owned()));
            }
            segments.extend(["namespaces", ns]);
        }
        segments.push(kind.plural());
        Ok(self.endpoint(&segments))
    }

    fn object_url(&self, object: &ObjectRef) -> Result<Url> {
        if !is_valid_name(&object.name) {
            return Err(KubeError::InvalidName(object.name.clone()));
        }
        let namespace = match (object.kind.namespaced(), object.namespace.as_deref()) {
            (true, Some(ns)) => Some(ns),
            (true, None) => return Err(KubeError::InvalidName(String::new())),
            (false, _) => None,
        };
        let mut url = self.collection_url(object.kind, namespace)?;
        url.path_segments_mut()
            .map_err(|()| KubeError::InvalidName(object.name.clone()))?
            .push(&object.name);
        Ok(url)
    }

    /// The base address with `segments` appended, each percent-encoded.
    fn endpoint(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", self.token.expose()))
            .unwrap_or_else(|_| HeaderValue::from_static(""));
        value.set_sensitive(true);
        request
            .header(AUTHORIZATION, value)
            .header(ACCEPT, "application/json")
    }

    fn get_json(&self, url: Url) -> Result<Value> {
        read_json(self.send(self.authorized(self.http.get(url)))?)
    }

    fn send(&self, request: RequestBuilder) -> Result<Response> {
        let response = request.send().map_err(transport_error)?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let message = read_body(response)
            .ok()
            .and_then(|body| status_message(&body))
            .unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("unknown error")
                    .to_owned()
            });
        Err(match status {
            StatusCode::UNAUTHORIZED => KubeError::Unauthorized,
            StatusCode::FORBIDDEN => KubeError::Forbidden(message),
            StatusCode::NOT_FOUND => KubeError::NotFound(message),
            StatusCode::CONFLICT => KubeError::Conflict(message),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                KubeError::Invalid(message)
            }
            s if s.is_redirection() => KubeError::Api {
                status: s.as_u16(),
                message: "the server tried to redirect; check the API address".into(),
            },
            s => KubeError::Api {
                status: s.as_u16(),
                message,
            },
        })
    }
}

fn transport_error(error: reqwest::Error) -> KubeError {
    if error.is_timeout() {
        return KubeError::Timeout;
    }
    // The innermost cause carries the useful part ("certificate unknown",
    // "connection refused"), not reqwest's generic wrapper.
    let mut source: &dyn std::error::Error = &error;
    while let Some(inner) = source.source() {
        source = inner;
    }
    KubeError::Connect(source.to_string())
}

fn read_body(response: Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .take(MAX_BODY + 1)
        .read_to_end(&mut body)
        .map_err(|e| KubeError::Decode(e.to_string()))?;
    if body.len() as u64 > MAX_BODY {
        return Err(KubeError::Decode("the response is too large".into()));
    }
    Ok(body)
}

fn read_json(response: Response) -> Result<Value> {
    let body = read_body(response)?;
    serde_json::from_slice(&body).map_err(|e| KubeError::Decode(e.to_string()))
}

/// The `message` of a Kubernetes `Status` object, if the body is one.
fn status_message(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let message = value.get("message")?.as_str()?.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{Cluster, ClusterId, parse_api_url};

    fn client(url: &str) -> KubeClient {
        KubeClient::new(&Credentials {
            cluster: Cluster {
                id: ClusterId(1),
                name: "test".into(),
                api_url: parse_api_url(url).unwrap(),
                description: String::new(),
                tls: TlsVerification::System,
                created_at: String::new(),
                updated_at: String::new(),
            },
            token: Secret::new("token"),
        })
        .unwrap()
    }

    fn object(kind: ResourceKind, namespace: Option<&str>, name: &str) -> ObjectRef {
        ObjectRef {
            kind,
            namespace: namespace.map(Into::into),
            name: name.into(),
        }
    }

    #[test]
    fn collection_urls() {
        let c = client("https://k8s.example.com:6443");
        let url = |kind, ns| c.collection_url(kind, ns).unwrap().to_string();
        assert_eq!(
            url(ResourceKind::Pods, None),
            "https://k8s.example.com:6443/api/v1/pods"
        );
        assert_eq!(
            url(ResourceKind::Deployments, Some("default")),
            "https://k8s.example.com:6443/apis/apps/v1/namespaces/default/deployments"
        );
        // Cluster-scoped kinds ignore the namespace.
        assert_eq!(
            url(ResourceKind::Nodes, Some("default")),
            "https://k8s.example.com:6443/api/v1/nodes"
        );
    }

    #[test]
    fn a_path_prefix_on_the_server_is_kept() {
        let c = client("https://rancher.example.com/k8s/clusters/c-1/");
        assert_eq!(
            c.collection_url(ResourceKind::Pods, Some("web"))
                .unwrap()
                .as_str(),
            "https://rancher.example.com/k8s/clusters/c-1/api/v1/namespaces/web/pods"
        );
    }

    #[test]
    fn object_urls() {
        let c = client("https://k8s.example.com");
        assert_eq!(
            c.object_url(&object(ResourceKind::Services, Some("web"), "api"))
                .unwrap()
                .as_str(),
            "https://k8s.example.com/api/v1/namespaces/web/services/api"
        );
        assert_eq!(
            c.object_url(&object(ResourceKind::Nodes, None, "node-1"))
                .unwrap()
                .as_str(),
            "https://k8s.example.com/api/v1/nodes/node-1"
        );
    }

    #[test]
    fn crafted_names_never_reach_the_url() {
        let c = client("https://k8s.example.com");
        for name in ["../../secrets", "a/b", "a?b", "a#b", "%2e%2e"] {
            assert!(matches!(
                c.object_url(&object(ResourceKind::Pods, Some("default"), name)),
                Err(KubeError::InvalidName(_))
            ));
            assert!(matches!(
                c.object_url(&object(ResourceKind::Pods, Some(name), "pod")),
                Err(KubeError::InvalidName(_))
            ));
        }
    }

    #[test]
    fn namespaced_objects_need_a_namespace() {
        let c = client("https://k8s.example.com");
        assert!(
            c.object_url(&object(ResourceKind::Pods, None, "p"))
                .is_err()
        );
    }

    #[test]
    fn status_messages_are_extracted() {
        let body =
            br#"{"kind":"Status","status":"Failure","message":"pods \"x\" not found","code":404}"#;
        assert_eq!(
            status_message(body).as_deref(),
            Some("pods \"x\" not found")
        );
        assert_eq!(status_message(b"<html>"), None);
    }

    #[test]
    fn debug_output_hides_the_token() {
        let printed = format!("{:?}", client("https://k8s.example.com"));
        assert!(!printed.contains("token"));
    }

    #[test]
    fn a_bad_ca_is_reported() {
        let mut creds = Credentials {
            cluster: client_cluster(),
            token: Secret::new("t"),
        };
        creds.cluster.tls = TlsVerification::CustomCa("not a pem".into());
        assert!(matches!(
            KubeClient::new(&creds),
            Err(KubeError::InvalidCertificate(_))
        ));
    }

    fn client_cluster() -> Cluster {
        Cluster {
            id: ClusterId(1),
            name: "t".into(),
            api_url: parse_api_url("https://k8s.example.com").unwrap(),
            description: String::new(),
            tls: TlsVerification::System,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}
