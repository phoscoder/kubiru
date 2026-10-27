//! Cluster connection settings and the validation applied before they are saved.

use std::fmt;
use std::net::IpAddr;

use url::{Host, Url};

use crate::secret::Secret;

/// Longest cluster name accepted, in characters.
pub const NAME_MAX: usize = 63;
/// Longest description accepted, in characters.
pub const DESCRIPTION_MAX: usize = 500;
/// Upper bound on a bearer token. Service-account JWTs are a few KiB at most.
pub const TOKEN_MAX: usize = 16 * 1024;
/// Upper bound on a PEM bundle pasted into the CA field.
pub const CA_MAX: usize = 64 * 1024;

/// Primary key of a saved cluster.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClusterId(pub i64);

impl fmt::Display for ClusterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How the API server's TLS certificate is checked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TlsVerification {
    /// Verify against the operating system's trusted roots.
    #[default]
    System,
    /// Verify against the given PEM-encoded certificate authority.
    CustomCa(String),
    /// Do not verify. Exposes the token to anyone who can intercept traffic.
    Insecure,
}

impl TlsVerification {
    pub fn is_insecure(&self) -> bool {
        matches!(self, Self::Insecure)
    }
}

/// A saved cluster. The bearer token is not part of it: it lives in the
/// operating system's keychain under the cluster's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cluster {
    pub id: ClusterId,
    pub name: String,
    pub api_url: Url,
    pub description: String,
    pub tls: TlsVerification,
    pub created_at: String,
    pub updated_at: String,
}

/// Raw form values, exactly as typed.
#[derive(Clone, Debug, Default)]
pub struct ClusterDraft {
    pub name: String,
    pub api_url: String,
    pub description: String,
    /// Blank while editing means "keep the stored token".
    pub token: Secret,
    pub ca_certificate: String,
    pub insecure_skip_tls_verify: bool,
}

/// A draft that passed [`ClusterDraft::validate`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidCluster {
    pub name: String,
    pub api_url: Url,
    pub description: String,
    /// `None` only when editing and the token field was left blank.
    pub token: Option<Secret>,
    pub tls: TlsVerification,
}

/// The form field a validation message belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Name,
    ApiUrl,
    Description,
    Token,
    CaCertificate,
}

/// Every problem found in a draft, so the form can show them all at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidationErrors(Vec<(Field, String)>);

impl ValidationErrors {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The message for `field`, if it has one.
    pub fn get(&self, field: Field) -> Option<&str> {
        self.0
            .iter()
            .find(|(f, _)| *f == field)
            .map(|(_, m)| m.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = &(Field, String)> {
        self.0.iter()
    }

    fn push(&mut self, field: Field, message: impl Into<String>) {
        self.0.push((field, message.into()));
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let messages: Vec<&str> = self.0.iter().map(|(_, m)| m.as_str()).collect();
        f.write_str(&messages.join("; "))
    }
}

impl std::error::Error for ValidationErrors {}

/// Whether the token field may be left blank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenRequirement {
    Required,
    /// Editing an existing cluster: blank keeps the stored token.
    KeepIfBlank,
}

impl ClusterDraft {
    /// Check every field and normalise the values that pass.
    pub fn validate(&self, token: TokenRequirement) -> Result<ValidCluster, ValidationErrors> {
        let mut errors = ValidationErrors::default();

        let name = self.name.trim().to_owned();
        if name.is_empty() {
            errors.push(Field::Name, "Enter a name for the cluster.");
        } else if name.chars().count() > NAME_MAX {
            errors.push(Field::Name, format!("Use at most {NAME_MAX} characters."));
        } else if name.chars().any(char::is_control) {
            errors.push(Field::Name, "The name cannot contain control characters.");
        }

        let api_url = match parse_api_url(&self.api_url) {
            Ok(url) => Some(url),
            Err(message) => {
                errors.push(Field::ApiUrl, message);
                None
            }
        };

        let description = self.description.trim().to_owned();
        if description.chars().count() > DESCRIPTION_MAX {
            errors.push(
                Field::Description,
                format!("Use at most {DESCRIPTION_MAX} characters."),
            );
        }

        let token_value = if self.token.is_empty() {
            if token == TokenRequirement::Required {
                errors.push(Field::Token, "Paste a bearer token for the cluster.");
            }
            None
        } else if self.token.expose().len() > TOKEN_MAX {
            errors.push(
                Field::Token,
                "This token is too long to be a Kubernetes bearer token.",
            );
            None
        } else if self
            .token
            .expose()
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        {
            errors.push(
                Field::Token,
                "The token cannot contain spaces or line breaks.",
            );
            None
        } else {
            Some(self.token.clone())
        };

        let ca = self.ca_certificate.trim();
        let tls = if ca.is_empty() {
            if self.insecure_skip_tls_verify {
                TlsVerification::Insecure
            } else {
                TlsVerification::System
            }
        } else if self.insecure_skip_tls_verify {
            errors.push(
                Field::CaCertificate,
                "Remove the CA certificate or turn off \u{201c}Skip TLS verification\u{201d}.",
            );
            TlsVerification::System
        } else if ca.len() > CA_MAX {
            errors.push(
                Field::CaCertificate,
                "This certificate bundle is too large.",
            );
            TlsVerification::System
        } else if !ca.contains("-----BEGIN CERTIFICATE-----") {
            errors.push(
                Field::CaCertificate,
                "Paste a PEM certificate (-----BEGIN CERTIFICATE-----).",
            );
            TlsVerification::System
        } else {
            TlsVerification::CustomCa(ca.to_owned())
        };

        match api_url {
            Some(api_url) if errors.is_empty() => Ok(ValidCluster {
                name,
                api_url,
                description,
                token: token_value,
                tls,
            }),
            _ => Err(errors),
        }
    }
}

/// Parse and normalise an API server address.
///
/// HTTPS is required, except for loopback addresses so `kubectl proxy`
/// keeps working. Credentials, queries and fragments are rejected because the
/// client appends API paths to whatever is stored.
pub fn parse_api_url(input: &str) -> Result<Url, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter the API server address, e.g. https://10.0.0.1:6443.".into());
    }
    let mut url = Url::parse(input).map_err(|_| {
        "Enter a full address including https://, e.g. https://10.0.0.1:6443.".to_string()
    })?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&url) => {}
        "http" => {
            return Err(
                "Use https://. Plain http is only allowed for localhost (kubectl proxy).".into(),
            );
        }
        _ => return Err("The address must start with https://.".into()),
    }
    if url.host().is_none() {
        return Err("The address is missing a host name.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Do not put credentials in the address; use the token field.".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Remove the query string or fragment from the address.".into());
    }
    let trimmed = url.path().trim_end_matches('/').to_owned();
    url.set_path(&trimmed);
    Ok(url)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ClusterDraft {
        ClusterDraft {
            name: "  Production ".into(),
            api_url: "https://10.0.0.1:6443/".into(),
            description: "Main cluster".into(),
            token: Secret::new("abc.def.ghi"),
            ca_certificate: String::new(),
            insecure_skip_tls_verify: false,
        }
    }

    #[test]
    fn a_complete_draft_is_normalised() {
        let valid = draft().validate(TokenRequirement::Required).expect("valid");
        assert_eq!(valid.name, "Production");
        assert_eq!(valid.api_url.as_str(), "https://10.0.0.1:6443/");
        assert_eq!(valid.tls, TlsVerification::System);
        assert_eq!(
            valid.token.as_ref().map(Secret::expose),
            Some("abc.def.ghi")
        );
    }

    #[test]
    fn all_problems_are_reported_together() {
        let bad = ClusterDraft {
            name: " ".into(),
            api_url: "ftp://x".into(),
            ..Default::default()
        };
        let errors = bad.validate(TokenRequirement::Required).unwrap_err();
        assert!(errors.get(Field::Name).is_some());
        assert!(errors.get(Field::ApiUrl).is_some());
        assert!(errors.get(Field::Token).is_some());
    }

    #[test]
    fn a_blank_token_is_fine_when_editing() {
        let edit = ClusterDraft {
            token: Secret::new(""),
            ..draft()
        };
        let valid = edit.validate(TokenRequirement::KeepIfBlank).expect("valid");
        assert!(valid.token.is_none());
    }

    #[test]
    fn tokens_with_whitespace_are_rejected() {
        let edit = ClusterDraft {
            token: Secret::new("abc def"),
            ..draft()
        };
        let errors = edit.validate(TokenRequirement::Required).unwrap_err();
        assert!(errors.get(Field::Token).is_some());
    }

    #[test]
    fn plain_http_is_only_allowed_on_loopback() {
        assert!(parse_api_url("http://127.0.0.1:8001").is_ok());
        assert!(parse_api_url("http://localhost:8001").is_ok());
        assert!(parse_api_url("http://[::1]:8001").is_ok());
        assert!(parse_api_url("http://10.0.0.1:6443").is_err());
    }

    #[test]
    fn credentials_and_queries_are_rejected() {
        assert!(parse_api_url("https://user:pass@example.com").is_err());
        assert!(parse_api_url("https://example.com/?a=b").is_err());
        assert!(parse_api_url("https://example.com/#x").is_err());
        assert!(parse_api_url("example.com").is_err());
    }

    #[test]
    fn path_prefixes_are_kept_without_a_trailing_slash() {
        let url = parse_api_url("https://rancher.example.com/k8s/clusters/c-123/").unwrap();
        assert_eq!(url.path(), "/k8s/clusters/c-123");
    }

    #[test]
    fn a_ca_and_insecure_mode_are_mutually_exclusive() {
        let both = ClusterDraft {
            ca_certificate: "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----".into(),
            insecure_skip_tls_verify: true,
            ..draft()
        };
        let errors = both.validate(TokenRequirement::Required).unwrap_err();
        assert!(errors.get(Field::CaCertificate).is_some());
    }

    #[test]
    fn a_pem_ca_is_accepted() {
        let with_ca = ClusterDraft {
            ca_certificate: "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----\n".into(),
            ..draft()
        };
        let valid = with_ca.validate(TokenRequirement::Required).unwrap();
        assert!(matches!(valid.tls, TlsVerification::CustomCa(_)));
    }

    #[test]
    fn insecure_mode_is_explicit() {
        let insecure = ClusterDraft {
            insecure_skip_tls_verify: true,
            ..draft()
        };
        assert!(
            insecure
                .validate(TokenRequirement::Required)
                .unwrap()
                .tls
                .is_insecure()
        );
    }
}
