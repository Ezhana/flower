use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{FailureCode, NodeId, PlanFingerprint, WorkflowId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Start,
    Activity,
    Finish,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum BackoffPolicy {
    None,
    Fixed {
        delay_ms: u64,
    },
    Exponential {
        initial_delay_ms: u64,
        multiplier: u32,
        maximum_delay_ms: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub retryable_failure_codes: BTreeSet<FailureCode>,
    pub backoff: BackoffPolicy,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            retryable_failure_codes: BTreeSet::new(),
            backoff: BackoffPolicy::None,
        }
    }
}

impl RetryPolicy {
    pub fn validate(&self) -> bool {
        self.max_attempts >= 1
            && match self.backoff {
                BackoffPolicy::None | BackoffPolicy::Fixed { .. } => true,
                BackoffPolicy::Exponential {
                    initial_delay_ms,
                    multiplier,
                    maximum_delay_ms,
                } => multiplier >= 1 && maximum_delay_ms >= initial_delay_ms,
            }
    }

    pub fn delay_after_failure(&self, failed_attempt_number: u32) -> Option<u64> {
        if failed_attempt_number == 0 || !self.validate() {
            return None;
        }
        match self.backoff {
            BackoffPolicy::None => Some(0),
            BackoffPolicy::Fixed { delay_ms } => Some(delay_ms),
            BackoffPolicy::Exponential {
                initial_delay_ms,
                multiplier,
                maximum_delay_ms,
            } => {
                let mut delay = initial_delay_ms;
                for _ in 1..failed_attempt_number {
                    if delay >= maximum_delay_ms {
                        break;
                    }
                    delay = delay
                        .saturating_mul(u64::from(multiplier))
                        .min(maximum_delay_ms);
                }
                Some(delay.min(maximum_delay_ms))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct NodeIndex(u32);

impl NodeIndex {
    pub fn new(value: u32) -> Self {
        Self(value)
    }
    pub fn value(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanNode {
    pub id: NodeId,
    pub kind: NodeKind,
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReference {
    pub workflow_id: WorkflowId,
    pub fingerprint: PlanFingerprint,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum PlanConstructionError {
    #[error("linear plan must contain at least start and finish nodes")]
    TooShort,
    #[error("linear plan contains duplicate node `{node_id}`")]
    DuplicateNode { node_id: NodeId },
    #[error("linear plan node `{node_id}` has invalid kind `{kind:?}`")]
    InvalidNodeKind { node_id: NodeId, kind: NodeKind },
    #[error("linear plan node `{node_id}` has an invalid retry policy")]
    InvalidRetryPolicy { node_id: NodeId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutableWorkflowPlan {
    workflow_id: WorkflowId,
    fingerprint: PlanFingerprint,
    nodes: Vec<PlanNode>,
    successor_by_node: Vec<Option<NodeIndex>>,
    start_node: NodeIndex,
    finish_node: NodeIndex,
}

impl ExecutableWorkflowPlan {
    pub fn from_linear_path(
        workflow_id: WorkflowId,
        nodes: Vec<PlanNode>,
    ) -> Result<Self, PlanConstructionError> {
        if nodes.len() < 2 {
            return Err(PlanConstructionError::TooShort);
        }
        let mut node_ids = BTreeSet::new();
        for (index, node) in nodes.iter().enumerate() {
            if !node_ids.insert(node.id.clone()) {
                return Err(PlanConstructionError::DuplicateNode {
                    node_id: node.id.clone(),
                });
            }
            let expected_kind = if index == 0 {
                NodeKind::Start
            } else if index + 1 == nodes.len() {
                NodeKind::Finish
            } else {
                NodeKind::Activity
            };
            if node.kind != expected_kind {
                return Err(PlanConstructionError::InvalidNodeKind {
                    node_id: node.id.clone(),
                    kind: node.kind,
                });
            }
            let retry_policy_is_valid = match (&node.kind, &node.retry_policy) {
                (NodeKind::Activity, Some(policy)) => policy.validate(),
                (NodeKind::Start | NodeKind::Finish, None) => true,
                _ => false,
            };
            if !retry_policy_is_valid {
                return Err(PlanConstructionError::InvalidRetryPolicy {
                    node_id: node.id.clone(),
                });
            }
        }
        let start_node = NodeIndex::new(0);
        let finish_node = NodeIndex::new((nodes.len() - 1) as u32);
        let successor_by_node = (0..nodes.len())
            .map(|index| (index + 1 < nodes.len()).then(|| NodeIndex::new((index + 1) as u32)))
            .collect();
        let fingerprint = fingerprint(&workflow_id, &nodes);
        Ok(Self {
            workflow_id,
            fingerprint,
            nodes,
            successor_by_node,
            start_node,
            finish_node,
        })
    }

    pub fn workflow_id(&self) -> &WorkflowId {
        &self.workflow_id
    }
    pub fn fingerprint(&self) -> &PlanFingerprint {
        &self.fingerprint
    }
    pub fn reference(&self) -> PlanReference {
        PlanReference {
            workflow_id: self.workflow_id.clone(),
            fingerprint: self.fingerprint.clone(),
        }
    }
    pub fn nodes(&self) -> &[PlanNode] {
        &self.nodes
    }
    pub fn start_node(&self) -> NodeIndex {
        self.start_node
    }
    pub fn finish_node(&self) -> NodeIndex {
        self.finish_node
    }
    pub fn node(&self, index: NodeIndex) -> Option<&PlanNode> {
        self.nodes.get(index.0 as usize)
    }

    pub fn node_by_id(&self, id: &NodeId) -> Option<(NodeIndex, &PlanNode)> {
        self.nodes
            .iter()
            .enumerate()
            .find(|(_, node)| &node.id == id)
            .map(|(index, node)| (NodeIndex::new(index as u32), node))
    }

    pub fn successor_of(&self, index: NodeIndex) -> Option<NodeIndex> {
        self.successor_by_node
            .get(index.0 as usize)
            .copied()
            .flatten()
    }

    pub fn retry_policy_for(&self, node_id: &NodeId) -> Option<&RetryPolicy> {
        self.node_by_id(node_id)
            .and_then(|(_, node)| node.retry_policy.as_ref())
    }

    pub fn validate_integrity(&self) -> bool {
        if self.nodes.len() < 2 {
            return false;
        }
        self.nodes
            .first()
            .is_some_and(|node| node.kind == NodeKind::Start)
            && self
                .nodes
                .last()
                .is_some_and(|node| node.kind == NodeKind::Finish)
            && self.nodes[1..self.nodes.len() - 1].iter().all(|node| {
                node.kind == NodeKind::Activity
                    && node
                        .retry_policy
                        .as_ref()
                        .is_some_and(RetryPolicy::validate)
            })
            && self
                .nodes
                .first()
                .is_some_and(|node| node.retry_policy.is_none())
            && self
                .nodes
                .last()
                .is_some_and(|node| node.retry_policy.is_none())
            && self
                .nodes
                .iter()
                .map(|node| &node.id)
                .collect::<BTreeSet<_>>()
                .len()
                == self.nodes.len()
            && self.start_node == NodeIndex::new(0)
            && self.finish_node == NodeIndex::new((self.nodes.len() - 1) as u32)
            && self.fingerprint == fingerprint(&self.workflow_id, &self.nodes)
            && self.successor_by_node.len() == self.nodes.len()
            && self
                .successor_by_node
                .iter()
                .enumerate()
                .all(|(index, successor)| {
                    *successor
                        == (index + 1 < self.nodes.len())
                            .then(|| NodeIndex::new((index + 1) as u32))
                })
    }
}

fn fingerprint(workflow_id: &WorkflowId, nodes: &[PlanNode]) -> PlanFingerprint {
    let canonical = ("flower/plan/v1", workflow_id, nodes);
    let bytes = serde_json::to_vec(&canonical).expect("canonical plan serialization cannot fail");
    let digest = Sha256::digest(bytes);
    PlanFingerprint::from_sha256(format!("{digest:x}")).expect("SHA-256 is valid hexadecimal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_backoff_caps_before_and_after_integer_overflow() {
        let policy = RetryPolicy {
            max_attempts: u32::MAX,
            retryable_failure_codes: BTreeSet::new(),
            backoff: BackoffPolicy::Exponential {
                initial_delay_ms: u64::MAX / 2,
                multiplier: u32::MAX,
                maximum_delay_ms: u64::MAX - 1,
            },
        };

        assert_eq!(policy.delay_after_failure(1), Some(u64::MAX / 2));
        assert_eq!(policy.delay_after_failure(2), Some(u64::MAX - 1));
        assert_eq!(policy.delay_after_failure(u32::MAX), Some(u64::MAX - 1));
    }

    #[test]
    fn rejects_zero_attempts_and_invalid_exponential_ranges() {
        assert!(
            !RetryPolicy {
                max_attempts: 0,
                ..RetryPolicy::default()
            }
            .validate()
        );
        assert!(
            !RetryPolicy {
                max_attempts: 2,
                retryable_failure_codes: BTreeSet::new(),
                backoff: BackoffPolicy::Exponential {
                    initial_delay_ms: 10,
                    multiplier: 0,
                    maximum_delay_ms: 5,
                },
            }
            .validate()
        );
    }
}
