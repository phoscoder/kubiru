//! Showing objects as YAML or JSON, and checking edits before they are applied.

use serde_json::{Map, Value};

/// The text format a manifest is shown and edited in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Yaml,
    Json,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Yaml => "YAML",
            Format::Json => "JSON",
        }
    }

    /// The editor language used for highlighting. JSON is valid YAML, so the
    /// YAML grammar highlights both.
    pub fn language(self) -> &'static str {
        "yaml"
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("{format} syntax error: {message}")]
    Syntax {
        format: &'static str,
        message: String,
    },
    #[error("the manifest must be a single object")]
    NotAnObject,
    #[error(
        "`{field}` cannot be changed here (was \u{201c}{expected}\u{201d}, now \u{201c}{found}\u{201d})"
    )]
    Immutable {
        field: &'static str,
        expected: String,
        found: String,
    },
    #[error("could not format the manifest: {0}")]
    Format(String),
}

/// Fields hidden by default because they are noise when reading or editing,
/// as with `kubectl get -o yaml`.
const MANAGED_FIELDS: &str = "managedFields";

/// Render `object` in `format`, without `metadata.managedFields`.
pub fn render(object: &Value, format: Format) -> Result<String, ManifestError> {
    let mut object = object.clone();
    if let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove(MANAGED_FIELDS);
    }
    match format {
        Format::Yaml => {
            serde_norway::to_string(&object).map_err(|e| ManifestError::Format(e.to_string()))
        }
        Format::Json => serde_json::to_string_pretty(&object)
            .map(|mut s| {
                s.push('\n');
                s
            })
            .map_err(|e| ManifestError::Format(e.to_string())),
    }
}

/// Parse edited text.
pub fn parse(text: &str, format: Format) -> Result<Value, ManifestError> {
    let syntax = |message: String| ManifestError::Syntax {
        format: format.label(),
        message,
    };
    let value: Value = match format {
        Format::Yaml => serde_norway::from_str(text).map_err(|e| syntax(e.to_string()))?,
        Format::Json => serde_json::from_str(text).map_err(|e| syntax(e.to_string()))?,
    };
    if value.is_object() {
        Ok(value)
    } else {
        Err(ManifestError::NotAnObject)
    }
}

/// Re-render text in another format, for the YAML/JSON switch.
pub fn convert(text: &str, from: Format, to: Format) -> Result<String, ManifestError> {
    if from == to {
        return Ok(text.to_owned());
    }
    render(&parse(text, from)?, to)
}

/// Check an edited manifest against the object it was loaded from and return
/// the body to send.
///
/// The identity of the object (API version, kind, name, namespace, uid)
/// cannot change: an edit is never allowed to turn into a write to a
/// different object. If the edit dropped `resourceVersion`, the loaded one is
/// put back so a concurrent change is detected instead of overwritten.
pub fn prepare_update(original: &Value, edited: Value) -> Result<Value, ManifestError> {
    let Value::Object(mut edited) = edited else {
        return Err(ManifestError::NotAnObject);
    };
    for (field, pointer) in [
        ("apiVersion", "/apiVersion"),
        ("kind", "/kind"),
        ("metadata.name", "/metadata/name"),
        ("metadata.namespace", "/metadata/namespace"),
        ("metadata.uid", "/metadata/uid"),
    ] {
        let expected = original.pointer(pointer);
        let found = pointer_in(&edited, pointer);
        // Leaving out the uid is fine; changing it is not.
        if field == "metadata.uid" && found.is_none() {
            continue;
        }
        if expected != found {
            return Err(ManifestError::Immutable {
                field,
                expected: display(expected),
                found: display(found),
            });
        }
    }
    let metadata = edited
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or(ManifestError::NotAnObject)?;
    if !metadata.contains_key("resourceVersion")
        && let Some(version) = original.pointer("/metadata/resourceVersion")
    {
        metadata.insert("resourceVersion".into(), version.clone());
    }
    Ok(Value::Object(edited))
}

fn pointer_in<'a>(object: &'a Map<String, Value>, pointer: &str) -> Option<&'a Value> {
    let mut parts = pointer.trim_start_matches('/').split('/');
    let first = object.get(parts.next()?)?;
    parts.try_fold(first, |value, key| value.get(key))
}

fn display(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "(none)".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn deployment() -> Value {
        json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": "api",
                "namespace": "web",
                "uid": "1234",
                "resourceVersion": "42",
                "managedFields": [{"manager": "kubectl"}]
            },
            "spec": {"replicas": 2}
        })
    }

    #[test]
    fn rendering_hides_managed_fields_in_both_formats() {
        for format in [Format::Yaml, Format::Json] {
            let text = render(&deployment(), format).unwrap();
            assert!(!text.contains("managedFields"), "{format:?}");
            assert!(text.contains("replicas"));
        }
    }

    #[test]
    fn yaml_round_trips() {
        let text = render(&deployment(), Format::Yaml).unwrap();
        let back = parse(&text, Format::Yaml).unwrap();
        assert_eq!(back["spec"]["replicas"], 2);
        assert_eq!(back["metadata"]["name"], "api");
    }

    #[test]
    fn converting_between_formats() {
        let yaml = render(&deployment(), Format::Yaml).unwrap();
        let json = convert(&yaml, Format::Yaml, Format::Json).unwrap();
        assert!(json.trim_start().starts_with('{'));
        assert_eq!(parse(&json, Format::Json).unwrap()["kind"], "Deployment");
    }

    #[test]
    fn syntax_errors_name_the_format() {
        let err = parse("a: [", Format::Yaml).unwrap_err();
        assert!(matches!(err, ManifestError::Syntax { format: "YAML", .. }));
        assert_eq!(
            parse("- 1\n- 2", Format::Yaml).unwrap_err(),
            ManifestError::NotAnObject
        );
    }

    #[test]
    fn edits_to_the_spec_are_accepted() {
        let mut edited = deployment();
        edited["spec"]["replicas"] = json!(5);
        let body = prepare_update(&deployment(), edited).unwrap();
        assert_eq!(body["spec"]["replicas"], 5);
    }

    #[test]
    fn renaming_or_moving_is_refused() {
        for (pointer, value) in [
            ("/metadata/name", json!("other")),
            ("/metadata/namespace", json!("prod")),
            ("/kind", json!("StatefulSet")),
            ("/apiVersion", json!("apps/v2")),
            ("/metadata/uid", json!("9999")),
        ] {
            let mut edited = deployment();
            *edited.pointer_mut(pointer).unwrap() = value;
            assert!(
                matches!(
                    prepare_update(&deployment(), edited),
                    Err(ManifestError::Immutable { .. })
                ),
                "{pointer}"
            );
        }
    }

    #[test]
    fn removing_the_namespace_is_refused() {
        let mut edited = deployment();
        edited["metadata"]
            .as_object_mut()
            .unwrap()
            .remove("namespace");
        assert!(prepare_update(&deployment(), edited).is_err());
    }

    #[test]
    fn a_dropped_resource_version_is_restored() {
        let mut edited = deployment();
        let metadata = edited["metadata"].as_object_mut().unwrap();
        metadata.remove("resourceVersion");
        metadata.remove("uid");
        let body = prepare_update(&deployment(), edited).unwrap();
        assert_eq!(body["metadata"]["resourceVersion"], "42");
    }
}
