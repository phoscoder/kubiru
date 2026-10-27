#![allow(clippy::unwrap_used)]
//! Drives `KubeClient` against a tiny local HTTP server to check what is
//! actually sent over the wire and how responses are interpreted.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use kubiru_core::cluster::{Cluster, ClusterId, TlsVerification, parse_api_url};
use kubiru_core::kube::{KubeClient, KubeError, ObjectRef, ResourceKind};
use kubiru_core::{Credentials, Secret};
use serde_json::json;

/// A request as the server saw it.
#[derive(Debug)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Serve one canned response per request, in order, and report each request.
fn serve(responses: Vec<(u16, &'static str, String)>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, extra_headers, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_owned();
            let path = parts.next().unwrap_or_default().to_owned();
            let mut headers = Vec::new();
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    headers.push((k.trim().to_owned(), v.trim().to_owned()));
                }
            }
            let length: usize = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0);
            let mut request_body = vec![0; length];
            reader.read_exact(&mut request_body).unwrap();
            let body_text = String::from_utf8_lossy(&request_body).into_owned();
            tx.send(Seen {
                method,
                path,
                headers,
                body: body_text,
            })
            .ok();
            let response = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra_headers}\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    (address, rx)
}

fn client(address: &str) -> KubeClient {
    KubeClient::new(&Credentials {
        cluster: Cluster {
            id: ClusterId(1),
            name: "local".into(),
            api_url: parse_api_url(address).unwrap(),
            description: String::new(),
            tls: TlsVerification::System,
            created_at: String::new(),
            updated_at: String::new(),
        },
        token: Secret::new("s3cr3t-token"),
    })
    .unwrap()
}

#[test]
fn listing_sends_the_bearer_token_and_returns_items() {
    let body = json!({"kind": "PodList", "items": [{"metadata": {"name": "a"}}, {"metadata": {"name": "b"}}]});
    let (address, seen) = serve(vec![(200, "", body.to_string())]);
    let items = client(&address)
        .list(ResourceKind::Pods, Some("default"))
        .unwrap();
    assert_eq!(items.len(), 2);
    let request = seen.recv().unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/v1/namespaces/default/pods");
    assert_eq!(request.header("authorization"), Some("Bearer s3cr3t-token"));
    assert_eq!(request.header("accept"), Some("application/json"));
    assert!(request.header("user-agent").unwrap().starts_with("kubiru/"));
}

#[test]
fn status_codes_map_to_errors_with_the_server_message() {
    let status = |code: u16, message: &str| {
        (
            code,
            "",
            json!({"kind": "Status", "message": message, "code": code}).to_string(),
        )
    };
    let (address, _seen) = serve(vec![
        status(401, "Unauthorized"),
        status(403, "pods is forbidden: User cannot list"),
        status(404, "pods \"x\" not found"),
        status(409, "the object has been modified"),
        status(422, "spec.replicas: Invalid value"),
        status(500, "etcd unavailable"),
    ]);
    let c = client(&address);
    let pod = ObjectRef {
        kind: ResourceKind::Pods,
        namespace: Some("default".into()),
        name: "x".into(),
    };
    assert!(matches!(c.get(&pod), Err(KubeError::Unauthorized)));
    assert!(matches!(c.get(&pod), Err(KubeError::Forbidden(m)) if m.contains("forbidden")));
    assert!(matches!(c.get(&pod), Err(KubeError::NotFound(m)) if m.contains("not found")));
    assert!(matches!(c.get(&pod), Err(KubeError::Conflict(_))));
    assert!(matches!(c.get(&pod), Err(KubeError::Invalid(m)) if m.contains("replicas")));
    let err = c.get(&pod).unwrap_err();
    assert!(err.is_transient());
    assert!(err.to_string().contains("etcd unavailable"));
}

#[test]
fn redirects_are_not_followed() {
    let (address, seen) = serve(vec![(
        302,
        "Location: http://127.0.0.1:1/steal\r\n",
        String::new(),
    )]);
    let err = client(&address)
        .list(ResourceKind::Nodes, None)
        .unwrap_err();
    assert!(matches!(err, KubeError::Api { status: 302, .. }), "{err:?}");
    assert_eq!(seen.recv().unwrap().path, "/api/v1/nodes");
    assert!(seen.try_recv().is_err(), "only one request was made");
}

#[test]
fn replace_puts_the_manifest_as_json() {
    let manifest = json!({"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "cfg", "namespace": "web"}});
    let (address, seen) = serve(vec![(200, "", manifest.to_string())]);
    let object = ObjectRef {
        kind: ResourceKind::ConfigMaps,
        namespace: Some("web".into()),
        name: "cfg".into(),
    };
    client(&address).replace(&object, &manifest).unwrap();
    let request = seen.recv().unwrap();
    assert_eq!(request.method, "PUT");
    assert_eq!(request.path, "/api/v1/namespaces/web/configmaps/cfg");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&request.body).unwrap(),
        manifest
    );
}

#[test]
fn delete_uses_background_propagation() {
    let (address, seen) = serve(vec![(200, "", "{}".into())]);
    let object = ObjectRef {
        kind: ResourceKind::Deployments,
        namespace: Some("web".into()),
        name: "api".into(),
    };
    client(&address).delete(&object).unwrap();
    let request = seen.recv().unwrap();
    assert_eq!(request.method, "DELETE");
    assert_eq!(request.path, "/apis/apps/v1/namespaces/web/deployments/api");
    assert!(
        request
            .body
            .contains("\"propagationPolicy\":\"Background\"")
    );
}

#[test]
fn verify_checks_version_and_token() {
    let (address, seen) = serve(vec![
        (
            200,
            "",
            json!({"gitVersion": "v1.34.1", "platform": "linux/amd64"}).to_string(),
        ),
        (200, "", json!({"items": []}).to_string()),
    ]);
    let version = client(&address).verify().unwrap();
    assert_eq!(version.git_version, "v1.34.1");
    assert_eq!(seen.recv().unwrap().path, "/version");
    assert_eq!(seen.recv().unwrap().path, "/api/v1/namespaces");
}

#[test]
fn an_unreachable_server_is_a_connection_error() {
    // Port 1 on loopback is never listening.
    let err = client("http://127.0.0.1:1")
        .list(ResourceKind::Pods, None)
        .unwrap_err();
    assert!(matches!(err, KubeError::Connect(_)), "{err:?}");
    assert!(err.is_transient());
}
