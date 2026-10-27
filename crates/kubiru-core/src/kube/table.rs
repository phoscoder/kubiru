//! Turns raw API objects into table rows, with the columns `kubectl get`
//! shows for each kind.

use std::cmp::Ordering;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::resource::{ObjectRef, ResourceKind};

/// How a status cell should be coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Neutral,
    Good,
    Warning,
    Bad,
}

/// What a column holds, which decides alignment and sorting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    Text,
    Number,
    /// Coloured by the row's [`Tone`].
    Status,
    Age,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnSpec {
    pub title: &'static str,
    pub kind: ColumnKind,
    /// Preferred width in pixels.
    pub width: f32,
}

const fn col(title: &'static str, kind: ColumnKind, width: f32) -> ColumnSpec {
    ColumnSpec { title, kind, width }
}

const NAME: ColumnSpec = col("Name", ColumnKind::Text, 280.);
const NAMESPACE: ColumnSpec = col("Namespace", ColumnKind::Text, 150.);
const AGE: ColumnSpec = col("Age", ColumnKind::Age, 80.);

/// A sortable cell value.
#[derive(Clone, Debug, PartialEq)]
pub enum SortKey {
    Text(String),
    Number(f64),
    /// Seconds since the epoch; larger is newer.
    Time(i64),
    Missing,
}

impl SortKey {
    pub fn compare(&self, other: &Self) -> Ordering {
        use SortKey::*;
        match (self, other) {
            (Number(a), Number(b)) => a.total_cmp(b),
            (Time(a), Time(b)) => a.cmp(b),
            (Text(a), Text(b)) => natural_cmp(a, b),
            (Missing, Missing) => Ordering::Equal,
            // Empty cells sort last in ascending order.
            (Missing, _) => Ordering::Greater,
            (_, Missing) => Ordering::Less,
            (a, b) => a.rank().cmp(&b.rank()),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            SortKey::Number(_) => 0,
            SortKey::Time(_) => 1,
            SortKey::Text(_) => 2,
            SortKey::Missing => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub text: String,
    pub sort: SortKey,
}

impl Cell {
    fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        if text.is_empty() {
            return Self::missing();
        }
        Self {
            sort: SortKey::Text(text.to_lowercase()),
            text,
        }
    }

    fn number(n: i64) -> Self {
        Self {
            text: n.to_string(),
            sort: SortKey::Number(n as f64),
        }
    }

    fn missing() -> Self {
        Self {
            text: "\u{2014}".into(),
            sort: SortKey::Missing,
        }
    }

    /// A cell whose text is shown as-is but which sorts by `key`.
    fn with_key(text: impl Into<String>, key: f64) -> Self {
        Self {
            text: text.into(),
            sort: SortKey::Number(key),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub object: ObjectRef,
    pub tone: Tone,
    pub cells: Vec<Cell>,
}

impl Row {
    /// Whether `query` (already lower-cased) appears in any cell.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self
                .cells
                .iter()
                .any(|c| c.text.to_lowercase().contains(query))
    }
}

/// The columns shown for `kind`.
pub fn columns(kind: ResourceKind) -> Vec<ColumnSpec> {
    use ColumnKind::*;
    let specific: &[ColumnSpec] = match kind {
        ResourceKind::Pods => &[
            col("Ready", Text, 70.),
            col("Status", Status, 170.),
            col("Restarts", Number, 80.),
            col("Node", Text, 180.),
            col("IP", Text, 120.),
        ],
        ResourceKind::Deployments => &[
            col("Ready", Text, 80.),
            col("Up-to-date", Number, 90.),
            col("Available", Number, 90.),
            col("Strategy", Text, 130.),
        ],
        ResourceKind::StatefulSets => &[col("Ready", Status, 80.), col("Service", Text, 160.)],
        ResourceKind::DaemonSets => &[
            col("Desired", Number, 80.),
            col("Current", Number, 80.),
            col("Ready", Status, 80.),
            col("Up-to-date", Number, 90.),
            col("Available", Number, 90.),
        ],
        ResourceKind::Jobs => &[col("Completions", Text, 110.), col("Status", Status, 110.)],
        ResourceKind::CronJobs => &[
            col("Schedule", Text, 130.),
            col("Suspended", Status, 90.),
            col("Active", Number, 70.),
            col("Last schedule", Age, 110.),
        ],
        ResourceKind::Services => &[
            col("Type", Text, 110.),
            col("Cluster IP", Text, 120.),
            col("External IP", Text, 140.),
            col("Ports", Text, 180.),
        ],
        ResourceKind::Ingresses => &[
            col("Class", Text, 110.),
            col("Hosts", Text, 220.),
            col("Address", Text, 140.),
        ],
        ResourceKind::ConfigMaps => &[col("Keys", Number, 70.)],
        ResourceKind::PersistentVolumeClaims => &[
            col("Status", Status, 90.),
            col("Volume", Text, 220.),
            col("Capacity", Text, 90.),
            col("Access modes", Text, 110.),
            col("Storage class", Text, 130.),
        ],
        ResourceKind::Nodes => &[
            col("Status", Status, 120.),
            col("Roles", Text, 140.),
            col("Version", Text, 110.),
            col("Internal IP", Text, 120.),
        ],
        ResourceKind::Namespaces => &[col("Status", Status, 100.)],
        ResourceKind::PersistentVolumes => &[
            col("Capacity", Text, 90.),
            col("Access modes", Text, 110.),
            col("Reclaim policy", Text, 120.),
            col("Status", Status, 90.),
            col("Claim", Text, 220.),
            col("Storage class", Text, 130.),
        ],
    };
    let mut all = vec![NAME];
    if kind.namespaced() {
        all.push(NAMESPACE);
    }
    all.extend_from_slice(specific);
    all.push(AGE);
    all
}

/// Build the row for one object. Returns `None` for objects without a name.
pub fn row(kind: ResourceKind, item: &Value, now: DateTime<Utc>) -> Option<Row> {
    let name = str_at(item, "/metadata/name")?.to_owned();
    let namespace = kind
        .namespaced()
        .then(|| str_at(item, "/metadata/namespace").unwrap_or("").to_owned());

    let mut cells = vec![Cell::text(&name)];
    if let Some(ns) = &namespace {
        cells.push(Cell::text(ns));
    }
    let (tone, specific) = match kind {
        ResourceKind::Pods => pod(item),
        ResourceKind::Deployments => deployment(item),
        ResourceKind::StatefulSets => stateful_set(item),
        ResourceKind::DaemonSets => daemon_set(item),
        ResourceKind::Jobs => job(item),
        ResourceKind::CronJobs => cron_job(item, now),
        ResourceKind::Services => service(item),
        ResourceKind::Ingresses => ingress(item),
        ResourceKind::ConfigMaps => config_map(item),
        ResourceKind::PersistentVolumeClaims => pvc(item),
        ResourceKind::Nodes => node(item),
        ResourceKind::Namespaces => namespace_row(item),
        ResourceKind::PersistentVolumes => pv(item),
    };
    cells.extend(specific);
    cells.push(age_cell(str_at(item, "/metadata/creationTimestamp"), now));

    Some(Row {
        object: ObjectRef {
            kind,
            namespace,
            name,
        },
        tone,
        cells,
    })
}

/// Rows for a whole list, in the API's order.
pub fn rows(kind: ResourceKind, items: &[Value], now: DateTime<Utc>) -> Vec<Row> {
    items
        .iter()
        .filter_map(|item| row(kind, item, now))
        .collect()
}

fn pod(item: &Value) -> (Tone, Vec<Cell>) {
    let statuses = array_at(item, "/status/containerStatuses");
    let total = array_at(item, "/spec/containers").len();
    let ready = statuses
        .iter()
        .filter(|s| s.get("ready").and_then(Value::as_bool) == Some(true))
        .count();
    let restarts: i64 = statuses
        .iter()
        .filter_map(|s| s.get("restartCount")?.as_i64())
        .sum();
    let status = pod_status(item);
    let tone = match status.as_str() {
        "Running" if ready == total => Tone::Good,
        "Succeeded" | "Completed" => Tone::Neutral,
        "Running" | "Pending" | "ContainerCreating" | "PodInitializing" | "Terminating" => {
            Tone::Warning
        }
        s if s.starts_with("Init:") && !s.contains("Err") && !s.contains("BackOff") => {
            Tone::Warning
        }
        _ => Tone::Bad,
    };
    let ready_cell = Cell::with_key(format!("{ready}/{total}"), ratio(ready, total));
    (
        tone,
        vec![
            ready_cell,
            Cell::text(status),
            Cell::number(restarts),
            Cell::text(str_at(item, "/spec/nodeName").unwrap_or("")),
            Cell::text(str_at(item, "/status/podIP").unwrap_or("")),
        ],
    )
}

/// The status `kubectl get pods` shows: the most telling container reason,
/// falling back to the phase.
pub fn pod_status(item: &Value) -> String {
    if item
        .pointer("/metadata/deletionTimestamp")
        .is_some_and(|v| !v.is_null())
    {
        return "Terminating".into();
    }
    let waiting_or_terminated = |s: &Value| {
        s.pointer("/state/waiting/reason")
            .or_else(|| s.pointer("/state/terminated/reason"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    for s in array_at(item, "/status/initContainerStatuses") {
        if s.pointer("/state/terminated/exitCode")
            .and_then(Value::as_i64)
            == Some(0)
        {
            continue;
        }
        if let Some(reason) = waiting_or_terminated(s) {
            return format!("Init:{reason}");
        }
    }
    let mut status = str_at(item, "/status/reason")
        .or_else(|| str_at(item, "/status/phase"))
        .unwrap_or("Unknown")
        .to_owned();
    for s in array_at(item, "/status/containerStatuses") {
        if let Some(reason) = waiting_or_terminated(s) {
            status = reason;
            if s.pointer("/state/waiting").is_some() {
                break;
            }
        }
    }
    status
}

fn deployment(item: &Value) -> (Tone, Vec<Cell>) {
    let desired = int_at(item, "/spec/replicas").unwrap_or(1);
    let ready = int_at(item, "/status/readyReplicas").unwrap_or(0);
    let tone = replica_tone(ready, desired);
    (
        tone,
        vec![
            Cell::with_key(
                format!("{ready}/{desired}"),
                ratio(ready as usize, desired as usize),
            ),
            Cell::number(int_at(item, "/status/updatedReplicas").unwrap_or(0)),
            Cell::number(int_at(item, "/status/availableReplicas").unwrap_or(0)),
            Cell::text(str_at(item, "/spec/strategy/type").unwrap_or("")),
        ],
    )
}

fn stateful_set(item: &Value) -> (Tone, Vec<Cell>) {
    let desired = int_at(item, "/spec/replicas").unwrap_or(1);
    let ready = int_at(item, "/status/readyReplicas").unwrap_or(0);
    (
        replica_tone(ready, desired),
        vec![
            Cell::with_key(
                format!("{ready}/{desired}"),
                ratio(ready as usize, desired as usize),
            ),
            Cell::text(str_at(item, "/spec/serviceName").unwrap_or("")),
        ],
    )
}

fn daemon_set(item: &Value) -> (Tone, Vec<Cell>) {
    let n = |p| int_at(item, p).unwrap_or(0);
    let desired = n("/status/desiredNumberScheduled");
    let ready = n("/status/numberReady");
    (
        replica_tone(ready, desired),
        vec![
            Cell::number(desired),
            Cell::number(n("/status/currentNumberScheduled")),
            Cell::number(ready),
            Cell::number(n("/status/updatedNumberScheduled")),
            Cell::number(n("/status/numberAvailable")),
        ],
    )
}

fn job(item: &Value) -> (Tone, Vec<Cell>) {
    let completions = int_at(item, "/spec/completions").unwrap_or(1);
    let succeeded = int_at(item, "/status/succeeded").unwrap_or(0);
    let condition = |kind: &str| {
        array_at(item, "/status/conditions").iter().any(|c| {
            c.get("type").and_then(Value::as_str) == Some(kind)
                && c.get("status").and_then(Value::as_str) == Some("True")
        })
    };
    let (tone, status) = if condition("Failed") {
        (Tone::Bad, "Failed")
    } else if condition("Complete") {
        (Tone::Good, "Complete")
    } else if item.pointer("/spec/suspend").and_then(Value::as_bool) == Some(true) {
        (Tone::Neutral, "Suspended")
    } else {
        (Tone::Warning, "Running")
    };
    (
        tone,
        vec![
            Cell::with_key(
                format!("{succeeded}/{completions}"),
                ratio(succeeded as usize, completions as usize),
            ),
            Cell::text(status),
        ],
    )
}

fn cron_job(item: &Value, now: DateTime<Utc>) -> (Tone, Vec<Cell>) {
    let suspended = item.pointer("/spec/suspend").and_then(Value::as_bool) == Some(true);
    (
        if suspended { Tone::Warning } else { Tone::Good },
        vec![
            Cell::text(str_at(item, "/spec/schedule").unwrap_or("")),
            Cell::text(if suspended { "Yes" } else { "No" }),
            Cell::number(array_at(item, "/status/active").len() as i64),
            age_cell(str_at(item, "/status/lastScheduleTime"), now),
        ],
    )
}

fn service(item: &Value) -> (Tone, Vec<Cell>) {
    let kind = str_at(item, "/spec/type").unwrap_or("ClusterIP");
    let mut external: Vec<String> = array_at(item, "/status/loadBalancer/ingress")
        .iter()
        .filter_map(|i| {
            i.get("ip")
                .or_else(|| i.get("hostname"))?
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    external.extend(strings_at(item, "/spec/externalIPs"));
    if kind == "ExternalName" {
        external.extend(str_at(item, "/spec/externalName").map(str::to_owned));
    }
    let external = if external.is_empty() && kind == "LoadBalancer" {
        "<pending>".to_owned()
    } else {
        external.join(", ")
    };
    let ports: Vec<String> = array_at(item, "/spec/ports")
        .iter()
        .map(|p| {
            let port = p.get("port").and_then(Value::as_i64).unwrap_or(0);
            let protocol = p.get("protocol").and_then(Value::as_str).unwrap_or("TCP");
            match p.get("nodePort").and_then(Value::as_i64) {
                Some(node) => format!("{port}:{node}/{protocol}"),
                None => format!("{port}/{protocol}"),
            }
        })
        .collect();
    (
        Tone::Neutral,
        vec![
            Cell::text(kind),
            Cell::text(str_at(item, "/spec/clusterIP").unwrap_or("")),
            Cell::text(external),
            Cell::text(ports.join(", ")),
        ],
    )
}

fn ingress(item: &Value) -> (Tone, Vec<Cell>) {
    let hosts: Vec<String> = array_at(item, "/spec/rules")
        .iter()
        .filter_map(|r| r.get("host")?.as_str().map(str::to_owned))
        .collect();
    let address: Vec<String> = array_at(item, "/status/loadBalancer/ingress")
        .iter()
        .filter_map(|i| {
            i.get("ip")
                .or_else(|| i.get("hostname"))?
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    (
        Tone::Neutral,
        vec![
            Cell::text(str_at(item, "/spec/ingressClassName").unwrap_or("")),
            Cell::text(if hosts.is_empty() {
                "*".to_owned()
            } else {
                hosts.join(", ")
            }),
            Cell::text(address.join(", ")),
        ],
    )
}

fn config_map(item: &Value) -> (Tone, Vec<Cell>) {
    let count = |p| {
        item.pointer(p)
            .and_then(Value::as_object)
            .map_or(0, |m| m.len())
    };
    (
        Tone::Neutral,
        vec![Cell::number((count("/data") + count("/binaryData")) as i64)],
    )
}

fn pvc(item: &Value) -> (Tone, Vec<Cell>) {
    let phase = str_at(item, "/status/phase").unwrap_or("Unknown");
    (
        phase_tone(phase, "Bound"),
        vec![
            Cell::text(phase),
            Cell::text(str_at(item, "/spec/volumeName").unwrap_or("")),
            Cell::text(str_at(item, "/status/capacity/storage").unwrap_or("")),
            Cell::text(access_modes(item, "/spec/accessModes")),
            Cell::text(str_at(item, "/spec/storageClassName").unwrap_or("")),
        ],
    )
}

fn node(item: &Value) -> (Tone, Vec<Cell>) {
    let ready = array_at(item, "/status/conditions")
        .iter()
        .find(|c| c.get("type").and_then(Value::as_str) == Some("Ready"))
        .and_then(|c| c.get("status")?.as_str());
    let unschedulable = item.pointer("/spec/unschedulable").and_then(Value::as_bool) == Some(true);
    let (tone, mut status) = match ready {
        Some("True") => (Tone::Good, "Ready".to_owned()),
        Some("False") => (Tone::Bad, "NotReady".to_owned()),
        _ => (Tone::Warning, "Unknown".to_owned()),
    };
    if unschedulable {
        status.push_str(",SchedulingDisabled");
    }
    let mut roles: Vec<&str> = item
        .pointer("/metadata/labels")
        .and_then(Value::as_object)
        .map(|labels| {
            labels
                .keys()
                .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
                .filter(|r| !r.is_empty())
                .collect()
        })
        .unwrap_or_default();
    roles.sort_unstable();
    let internal_ip = array_at(item, "/status/addresses")
        .iter()
        .find(|a| a.get("type").and_then(Value::as_str) == Some("InternalIP"))
        .and_then(|a| a.get("address")?.as_str())
        .unwrap_or("");
    (
        if unschedulable && tone == Tone::Good {
            Tone::Warning
        } else {
            tone
        },
        vec![
            Cell::text(status),
            Cell::text(roles.join(", ")),
            Cell::text(str_at(item, "/status/nodeInfo/kubeletVersion").unwrap_or("")),
            Cell::text(internal_ip),
        ],
    )
}

fn namespace_row(item: &Value) -> (Tone, Vec<Cell>) {
    let phase = str_at(item, "/status/phase").unwrap_or("Unknown");
    (phase_tone(phase, "Active"), vec![Cell::text(phase)])
}

fn pv(item: &Value) -> (Tone, Vec<Cell>) {
    let phase = str_at(item, "/status/phase").unwrap_or("Unknown");
    let claim = match (
        str_at(item, "/spec/claimRef/namespace"),
        str_at(item, "/spec/claimRef/name"),
    ) {
        (Some(ns), Some(name)) => format!("{ns}/{name}"),
        _ => String::new(),
    };
    (
        match phase {
            "Bound" | "Available" => Tone::Good,
            "Released" | "Pending" => Tone::Warning,
            _ => Tone::Bad,
        },
        vec![
            Cell::text(str_at(item, "/spec/capacity/storage").unwrap_or("")),
            Cell::text(access_modes(item, "/spec/accessModes")),
            Cell::text(str_at(item, "/spec/persistentVolumeReclaimPolicy").unwrap_or("")),
            Cell::text(phase),
            Cell::text(claim),
            Cell::text(str_at(item, "/spec/storageClassName").unwrap_or("")),
        ],
    )
}

fn replica_tone(ready: i64, desired: i64) -> Tone {
    if desired == 0 || ready >= desired {
        Tone::Good
    } else if ready == 0 {
        Tone::Bad
    } else {
        Tone::Warning
    }
}

fn phase_tone(phase: &str, healthy: &str) -> Tone {
    if phase == healthy {
        Tone::Good
    } else if phase == "Pending" || phase == "Terminating" {
        Tone::Warning
    } else {
        Tone::Bad
    }
}

fn access_modes(item: &Value, pointer: &str) -> String {
    strings_at(item, pointer)
        .iter()
        .map(|m| match m.as_str() {
            "ReadWriteOnce" => "RWO",
            "ReadOnlyMany" => "ROX",
            "ReadWriteMany" => "RWX",
            "ReadWriteOncePod" => "RWOP",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn age_cell(timestamp: Option<&str>, now: DateTime<Utc>) -> Cell {
    match timestamp.and_then(|t| DateTime::parse_from_rfc3339(t).ok()) {
        Some(created) => {
            let created = created.with_timezone(&Utc);
            Cell {
                text: format_age(now - created),
                sort: SortKey::Time(-created.timestamp()),
            }
        }
        None => Cell::missing(),
    }
}

/// Format a duration the way `kubectl` does: `45s`, `12m`, `5h`, `3d`, `2y`.
pub fn format_age(age: chrono::TimeDelta) -> String {
    let seconds = age.num_seconds().max(0);
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86_400);
    if seconds < 120 {
        format!("{seconds}s")
    } else if minutes < 10 {
        format!("{minutes}m{}s", seconds % 60)
    } else if hours < 3 {
        format!("{minutes}m")
    } else if hours < 8 {
        format!("{hours}h{}m", minutes % 60)
    } else if days < 2 {
        format!("{hours}h")
    } else if days < 8 {
        format!("{days}d{}h", hours % 24)
    } else if days < 365 * 2 {
        format!("{days}d")
    } else {
        format!("{}y", days / 365)
    }
}

/// Compare strings so that embedded numbers sort by value: `pod-2 < pod-10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

fn str_at<'a>(item: &'a Value, pointer: &str) -> Option<&'a str> {
    item.pointer(pointer)?.as_str()
}

fn int_at(item: &Value, pointer: &str) -> Option<i64> {
    item.pointer(pointer)?.as_i64()
}

fn array_at<'a>(item: &'a Value, pointer: &str) -> &'a [Value] {
    item.pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn strings_at(item: &Value, pointer: &str) -> Vec<String> {
    array_at(item, pointer)
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn texts(row: &Row) -> Vec<&str> {
        row.cells.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn every_kind_has_matching_columns_and_cells() {
        let item = json!({"metadata": {"name": "x", "namespace": "ns", "creationTimestamp": "2026-10-10T11:00:00Z"}});
        for kind in ResourceKind::ALL {
            let row = row(kind, &item, now()).unwrap();
            assert_eq!(row.cells.len(), columns(kind).len(), "{kind:?}");
        }
    }

    #[test]
    fn running_pod() {
        let pod = json!({
            "metadata": {"name": "web-1", "namespace": "default", "creationTimestamp": "2026-10-09T12:00:00Z"},
            "spec": {"nodeName": "node-a", "containers": [{}, {}]},
            "status": {"phase": "Running", "podIP": "10.1.0.4", "containerStatuses": [
                {"ready": true, "restartCount": 2, "state": {"running": {}}},
                {"ready": true, "restartCount": 1, "state": {"running": {}}}
            ]}
        });
        let row = row(ResourceKind::Pods, &pod, now()).unwrap();
        assert_eq!(
            texts(&row),
            [
                "web-1", "default", "2/2", "Running", "3", "node-a", "10.1.0.4", "24h"
            ]
        );
        assert_eq!(row.tone, Tone::Good);
        assert_eq!(row.object.namespace.as_deref(), Some("default"));
    }

    #[test]
    fn crash_looping_pod_shows_the_reason() {
        let pod = json!({
            "metadata": {"name": "bad"},
            "spec": {"containers": [{}]},
            "status": {"phase": "Running", "containerStatuses": [
                {"ready": false, "restartCount": 9, "state": {"waiting": {"reason": "CrashLoopBackOff"}}}
            ]}
        });
        let row = row(ResourceKind::Pods, &pod, now()).unwrap();
        assert_eq!(row.cells[3].text, "CrashLoopBackOff");
        assert_eq!(row.tone, Tone::Bad);
    }

    #[test]
    fn failing_init_container() {
        let pod = json!({
            "metadata": {"name": "p"},
            "status": {"phase": "Pending", "initContainerStatuses": [
                {"state": {"waiting": {"reason": "ImagePullBackOff"}}}
            ]}
        });
        assert_eq!(pod_status(&pod), "Init:ImagePullBackOff");
    }

    #[test]
    fn terminating_pod() {
        let pod = json!({"metadata": {"name": "p", "deletionTimestamp": "2026-10-10T11:59:00Z"}, "status": {"phase": "Running"}});
        assert_eq!(pod_status(&pod), "Terminating");
    }

    #[test]
    fn deployment_partially_ready() {
        let d = json!({
            "metadata": {"name": "api", "namespace": "web"},
            "spec": {"replicas": 3, "strategy": {"type": "RollingUpdate"}},
            "status": {"readyReplicas": 2, "updatedReplicas": 3, "availableReplicas": 2}
        });
        let row = row(ResourceKind::Deployments, &d, now()).unwrap();
        assert_eq!(texts(&row)[2..6], ["2/3", "3", "2", "RollingUpdate"]);
        assert_eq!(row.tone, Tone::Warning);
    }

    #[test]
    fn service_with_node_ports_and_pending_lb() {
        let s = json!({
            "metadata": {"name": "lb", "namespace": "web"},
            "spec": {"type": "LoadBalancer", "clusterIP": "10.96.0.10", "ports": [
                {"port": 80, "nodePort": 30080, "protocol": "TCP"}, {"port": 53, "protocol": "UDP"}
            ]}
        });
        let row = row(ResourceKind::Services, &s, now()).unwrap();
        assert_eq!(
            texts(&row)[2..6],
            [
                "LoadBalancer",
                "10.96.0.10",
                "<pending>",
                "80:30080/TCP, 53/UDP"
            ]
        );
    }

    #[test]
    fn node_roles_and_status() {
        let n = json!({
            "metadata": {"name": "cp-1", "labels": {"node-role.kubernetes.io/control-plane": ""}},
            "spec": {"unschedulable": true},
            "status": {
                "conditions": [{"type": "Ready", "status": "True"}],
                "nodeInfo": {"kubeletVersion": "v1.34.1"},
                "addresses": [{"type": "InternalIP", "address": "10.0.0.2"}]
            }
        });
        let row = row(ResourceKind::Nodes, &n, now()).unwrap();
        assert_eq!(
            texts(&row)[..5],
            [
                "cp-1",
                "Ready,SchedulingDisabled",
                "control-plane",
                "v1.34.1",
                "10.0.0.2"
            ]
        );
        assert_eq!(row.tone, Tone::Warning);
        assert!(row.object.namespace.is_none());
    }

    #[test]
    fn persistent_volume_claim_ref() {
        let pv = json!({
            "metadata": {"name": "pv-1"},
            "spec": {"capacity": {"storage": "10Gi"}, "accessModes": ["ReadWriteOnce"],
                     "persistentVolumeReclaimPolicy": "Retain", "claimRef": {"namespace": "db", "name": "data"}},
            "status": {"phase": "Bound"}
        });
        let row = row(ResourceKind::PersistentVolumes, &pv, now()).unwrap();
        assert_eq!(
            texts(&row)[1..6],
            ["10Gi", "RWO", "Retain", "Bound", "db/data"]
        );
    }

    #[test]
    fn objects_without_names_are_skipped() {
        assert!(row(ResourceKind::Pods, &json!({"metadata": {}}), now()).is_none());
    }

    #[test]
    fn ages_match_kubectl() {
        let cases = [
            (45, "45s"),
            (150, "2m30s"),
            (60 * 45, "45m"),
            (3600 * 5 + 60 * 7, "5h7m"),
            (3600 * 20, "20h"),
            (86_400 * 3 + 3600 * 4, "3d4h"),
            (86_400 * 40, "40d"),
            (86_400 * 800, "2y"),
        ];
        for (seconds, expected) in cases {
            assert_eq!(format_age(TimeDelta::seconds(seconds)), expected);
        }
        assert_eq!(format_age(TimeDelta::seconds(-5)), "0s");
    }

    #[test]
    fn natural_sorting() {
        let mut names = vec!["pod-10", "pod-2", "pod-1", "Pod-3"];
        names.sort_by(|a, b| natural_cmp(&a.to_lowercase(), &b.to_lowercase()));
        assert_eq!(names, ["pod-1", "pod-2", "Pod-3", "pod-10"]);
    }

    #[test]
    fn sort_keys_put_missing_values_last_and_newest_age_first() {
        assert_eq!(
            SortKey::Missing.compare(&SortKey::Number(1.)),
            Ordering::Greater
        );
        let new = age_cell(Some("2026-10-10T11:59:00Z"), now());
        let old = age_cell(Some("2026-10-01T00:00:00Z"), now());
        assert_eq!(
            new.sort.compare(&old.sort),
            Ordering::Less,
            "younger sorts first when ascending by age"
        );
    }

    #[test]
    fn rows_can_be_filtered() {
        let item = json!({"metadata": {"name": "Frontend", "namespace": "web"}});
        let row = row(ResourceKind::ConfigMaps, &item, now()).unwrap();
        assert!(row.matches("front"));
        assert!(row.matches("web"));
        assert!(!row.matches("backend"));
    }
}
