//! The numbers on the cluster overview.

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::resource::{ObjectRef, ResourceKind};
use super::table::{Tone, pod_status, row};

/// A count of healthy items out of a total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Health {
    pub healthy: usize,
    pub total: usize,
}

impl Health {
    pub fn tone(self) -> Tone {
        if self.total == 0 || self.healthy == self.total {
            Tone::Good
        } else if self.healthy == 0 {
            Tone::Bad
        } else {
            Tone::Warning
        }
    }
}

/// Pod counts by phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PodPhases {
    pub running: usize,
    pub pending: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub unknown: usize,
}

impl PodPhases {
    pub fn total(self) -> usize {
        self.running + self.pending + self.succeeded + self.failed + self.unknown
    }
}

/// A pod that needs attention.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub object: ObjectRef,
    pub status: String,
    pub restarts: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Overview {
    pub nodes: Health,
    pub deployments: Health,
    pub pods: PodPhases,
    pub problems: Vec<Problem>,
}

/// Most problem pods listed on the overview.
pub const PROBLEMS_SHOWN: usize = 8;

pub fn summarize(
    nodes: &[Value],
    deployments: &[Value],
    pods: &[Value],
    now: DateTime<Utc>,
) -> Overview {
    let health = |kind, items: &[Value]| Health {
        healthy: items
            .iter()
            .filter_map(|i| row(kind, i, now))
            .filter(|r| r.tone == Tone::Good)
            .count(),
        total: items.len(),
    };

    let mut phases = PodPhases::default();
    let mut problems = Vec::new();
    for pod in pods {
        match pod.pointer("/status/phase").and_then(Value::as_str) {
            Some("Running") => phases.running += 1,
            Some("Pending") => phases.pending += 1,
            Some("Succeeded") => phases.succeeded += 1,
            Some("Failed") => phases.failed += 1,
            _ => phases.unknown += 1,
        }
        if let Some(r) = row(ResourceKind::Pods, pod, now).filter(|r| r.tone == Tone::Bad) {
            let restarts = pod
                .pointer("/status/containerStatuses")
                .and_then(Value::as_array)
                .map_or(0, |s| {
                    s.iter()
                        .filter_map(|c| c.get("restartCount")?.as_i64())
                        .sum()
                });
            problems.push(Problem {
                object: r.object,
                status: pod_status(pod),
                restarts,
            });
        }
    }
    problems.sort_by(|a, b| {
        b.restarts
            .cmp(&a.restarts)
            .then_with(|| a.object.name.cmp(&b.object.name))
    });
    problems.truncate(PROBLEMS_SHOWN);

    Overview {
        nodes: health(ResourceKind::Nodes, nodes),
        deployments: health(ResourceKind::Deployments, deployments),
        pods: phases,
        problems,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_health_and_lists_problem_pods_by_restarts() {
        let nodes = [
            json!({"metadata": {"name": "a"}, "status": {"conditions": [{"type": "Ready", "status": "True"}]}}),
            json!({"metadata": {"name": "b"}, "status": {"conditions": [{"type": "Ready", "status": "False"}]}}),
        ];
        let deployments = [
            json!({"metadata": {"name": "d", "namespace": "x"}, "spec": {"replicas": 1}, "status": {"readyReplicas": 1}}),
        ];
        let crash = |name: &str, restarts: i64| {
            json!({
                "metadata": {"name": name, "namespace": "x"},
                "spec": {"containers": [{}]},
                "status": {"phase": "Running", "containerStatuses": [
                    {"ready": false, "restartCount": restarts, "state": {"waiting": {"reason": "CrashLoopBackOff"}}}
                ]}
            })
        };
        let pods = [
            crash("few", 2),
            crash("many", 30),
            json!({"metadata": {"name": "ok", "namespace": "x"}, "status": {"phase": "Succeeded"}}),
            json!({"metadata": {"name": "wait", "namespace": "x"}, "status": {"phase": "Pending"}}),
        ];
        let o = summarize(&nodes, &deployments, &pods, Utc::now());
        assert_eq!(
            o.nodes,
            Health {
                healthy: 1,
                total: 2
            }
        );
        assert_eq!(o.nodes.tone(), Tone::Warning);
        assert_eq!(o.deployments.tone(), Tone::Good);
        assert_eq!(
            o.pods,
            PodPhases {
                running: 2,
                pending: 1,
                succeeded: 1,
                failed: 0,
                unknown: 0
            }
        );
        let names: Vec<_> = o.problems.iter().map(|p| p.object.name.as_str()).collect();
        assert_eq!(names, ["many", "few"]);
        assert_eq!(o.problems[0].status, "CrashLoopBackOff");
    }

    #[test]
    fn an_empty_cluster_is_healthy() {
        let o = summarize(&[], &[], &[], Utc::now());
        assert_eq!(o.nodes.tone(), Tone::Good);
        assert_eq!(o.pods.total(), 0);
    }
}
