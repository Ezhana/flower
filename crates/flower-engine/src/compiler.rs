use std::collections::{BTreeMap, BTreeSet};

use crate::{EdgeId, ExecutableWorkflowPlan, NodeId, NodeKind, PlanNode, RetryPolicy, WorkflowId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDefinition {
    pub id: NodeId,
    pub kind: NodeKind,
    #[serde(default)]
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeDefinition {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub id: WorkflowId,
    pub nodes: Vec<NodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub subject: Option<String>,
}

impl Diagnostic {
    fn new(code: &str, message: impl Into<String>, subject: Option<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            subject,
        }
    }
}

pub fn compile(definition: WorkflowDefinition) -> Result<ExecutableWorkflowPlan, Vec<Diagnostic>> {
    let mut nodes = BTreeMap::new();
    for node in definition.nodes {
        let id = node.id.clone();
        match (&node.kind, &node.retry_policy) {
            (NodeKind::Activity, Some(policy)) if !policy.validate() => {
                return error(
                    "invalid-retry-policy",
                    format!("activity `{id}` has an invalid retry policy"),
                    id,
                );
            }
            (NodeKind::Start | NodeKind::Finish, Some(_)) => {
                return error(
                    "retry-policy-on-non-activity",
                    format!("non-activity node `{id}` cannot define a retry policy"),
                    id,
                );
            }
            _ => {}
        }
        if nodes.insert(id.clone(), node).is_some() {
            return error("duplicate-node", format!("duplicate node `{id}`"), id);
        }
    }

    let start = exactly_one(&nodes, NodeKind::Start, "start")?;
    let finish = exactly_one(&nodes, NodeKind::Finish, "finish")?;
    let mut edge_ids = BTreeSet::new();
    let mut incoming = BTreeMap::<NodeId, usize>::new();
    let mut outgoing = BTreeMap::<NodeId, usize>::new();
    let mut successors = BTreeMap::<NodeId, NodeId>::new();

    for edge in definition.edges {
        if !edge_ids.insert(edge.id.clone()) {
            return error(
                "duplicate-edge",
                format!("duplicate edge `{}`", edge.id),
                edge.id,
            );
        }
        for endpoint in [&edge.source, &edge.target] {
            if !nodes.contains_key(endpoint) {
                return error(
                    "missing-edge-endpoint",
                    format!("edge `{}` references missing node `{endpoint}`", edge.id),
                    endpoint,
                );
            }
        }
        *incoming.entry(edge.target.clone()).or_default() += 1;
        *outgoing.entry(edge.source.clone()).or_default() += 1;
        successors.insert(edge.source, edge.target);
    }

    for (id, node) in &nodes {
        let incoming_count = incoming.get(id).copied().unwrap_or_default();
        let outgoing_count = outgoing.get(id).copied().unwrap_or_default();
        let expected_incoming = usize::from(node.kind != NodeKind::Start);
        let expected_outgoing = usize::from(node.kind != NodeKind::Finish);
        if incoming_count != expected_incoming {
            let code = if incoming_count > 1 {
                "branching-without-gateway"
            } else {
                "invalid-incoming-degree"
            };
            return error(
                code,
                format!(
                    "node `{id}` has {incoming_count} incoming edges; expected {expected_incoming}"
                ),
                id,
            );
        }
        if outgoing_count != expected_outgoing {
            let code = if outgoing_count > 1 {
                "branching-without-gateway"
            } else {
                "invalid-outgoing-degree"
            };
            return error(
                code,
                format!(
                    "node `{id}` has {outgoing_count} outgoing edges; expected {expected_outgoing}"
                ),
                id,
            );
        }
    }

    let mut path = Vec::with_capacity(nodes.len());
    let mut visited = BTreeSet::new();
    let mut current = start.clone();
    loop {
        if !visited.insert(current.clone()) {
            return error("cycle", format!("cycle at node `{current}`"), current);
        }
        let node = nodes.get(&current).expect("path nodes were checked");
        path.push(PlanNode {
            id: node.id.clone(),
            kind: node.kind,
            retry_policy: (node.kind == NodeKind::Activity)
                .then(|| node.retry_policy.clone().unwrap_or_default()),
        });
        if current == finish {
            break;
        }
        current = successors
            .get(&current)
            .expect("degrees guarantee successor")
            .clone();
    }
    if visited.len() != nodes.len() {
        let disconnected = nodes
            .keys()
            .find(|id| !visited.contains(*id))
            .expect("node counts differ");
        return error(
            "disconnected-node",
            format!("node `{disconnected}` is disconnected"),
            disconnected,
        );
    }

    Ok(
        ExecutableWorkflowPlan::from_linear_path(definition.id, path)
            .expect("compiler validation guarantees a valid linear path"),
    )
}

fn exactly_one(
    nodes: &BTreeMap<NodeId, NodeDefinition>,
    kind: NodeKind,
    label: &str,
) -> Result<NodeId, Vec<Diagnostic>> {
    let matching = nodes
        .values()
        .filter(|node| node.kind == kind)
        .collect::<Vec<_>>();
    if matching.len() == 1 {
        return Ok(matching[0].id.clone());
    }
    Err(vec![Diagnostic::new(
        &format!("invalid-{label}-count"),
        format!(
            "workflow must have exactly one {label}; found {}",
            matching.len()
        ),
        None,
    )])
}

fn error<T>(code: &str, message: String, subject: impl ToString) -> Result<T, Vec<Diagnostic>> {
    Err(vec![Diagnostic::new(
        code,
        message,
        Some(subject.to_string()),
    )])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::{BackoffPolicy, FailureCode};

    use super::*;

    fn id<T: std::str::FromStr>(value: &str) -> T
    where
        T::Err: std::fmt::Debug,
    {
        value.parse().unwrap()
    }
    fn node(value: &str, kind: NodeKind) -> NodeDefinition {
        NodeDefinition {
            id: id(value),
            kind,
            retry_policy: None,
        }
    }
    fn edge(value: &str, source: &str, target: &str) -> EdgeDefinition {
        EdgeDefinition {
            id: id(value),
            source: id(source),
            target: id(target),
        }
    }

    #[test]
    fn fingerprint_ignores_definition_order() {
        let first = WorkflowDefinition {
            id: id("flow"),
            nodes: vec![
                node("start", NodeKind::Start),
                node("work", NodeKind::Activity),
                node("finish", NodeKind::Finish),
            ],
            edges: vec![edge("a", "start", "work"), edge("b", "work", "finish")],
        };
        let mut second = first.clone();
        second.nodes.reverse();
        second.edges.reverse();
        assert_eq!(
            compile(first).unwrap().fingerprint(),
            compile(second).unwrap().fingerprint()
        );
    }

    #[test]
    fn fingerprint_covers_retry_behavior() {
        let first = WorkflowDefinition {
            id: id("flow"),
            nodes: vec![
                node("start", NodeKind::Start),
                node("work", NodeKind::Activity),
                node("finish", NodeKind::Finish),
            ],
            edges: vec![edge("a", "start", "work"), edge("b", "work", "finish")],
        };
        let mut second = first.clone();
        second.nodes[1].retry_policy = Some(RetryPolicy {
            max_attempts: 2,
            retryable_failure_codes: BTreeSet::from([FailureCode::new("transient").unwrap()]),
            backoff: BackoffPolicy::Fixed { delay_ms: 100 },
        });

        assert_ne!(
            compile(first).unwrap().fingerprint(),
            compile(second).unwrap().fingerprint()
        );
    }

    #[test]
    fn rejects_branching_without_gateway() {
        let definition = WorkflowDefinition {
            id: id("flow"),
            nodes: vec![
                node("start", NodeKind::Start),
                node("one", NodeKind::Activity),
                node("two", NodeKind::Activity),
                node("finish", NodeKind::Finish),
            ],
            edges: vec![
                edge("a", "start", "one"),
                edge("b", "start", "two"),
                edge("c", "one", "finish"),
                edge("d", "two", "finish"),
            ],
        };
        assert_eq!(
            compile(definition).unwrap_err()[0].code,
            "branching-without-gateway"
        );
    }
}
