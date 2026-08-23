use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{NodeId, PlanFingerprint, WorkflowId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpecificationVersion {
    pub major: u16,
    pub minor: u16,
}

impl SpecificationVersion {
    pub const V0_1: Self = Self { major: 0, minor: 1 };
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Start,
    Activity,
    Finish,
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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanReference {
    pub specification_version: SpecificationVersion,
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
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutableWorkflowPlan {
    specification_version: SpecificationVersion,
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
        }
        let start_node = NodeIndex::new(0);
        let finish_node = NodeIndex::new((nodes.len() - 1) as u32);
        let successor_by_node = (0..nodes.len())
            .map(|index| (index + 1 < nodes.len()).then(|| NodeIndex::new((index + 1) as u32)))
            .collect();
        let fingerprint = fingerprint(&workflow_id, &nodes);
        Ok(Self {
            specification_version: SpecificationVersion::V0_1,
            workflow_id,
            fingerprint,
            nodes,
            successor_by_node,
            start_node,
            finish_node,
        })
    }

    pub fn specification_version(&self) -> SpecificationVersion {
        self.specification_version
    }
    pub fn workflow_id(&self) -> &WorkflowId {
        &self.workflow_id
    }
    pub fn fingerprint(&self) -> &PlanFingerprint {
        &self.fingerprint
    }
    pub fn reference(&self) -> PlanReference {
        PlanReference {
            specification_version: self.specification_version,
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

    pub fn activity_at_revision(&self, revision: u64) -> Option<&PlanNode> {
        let index = usize::try_from(revision).ok()?;
        let node = self.nodes.get(index)?;
        (node.kind == NodeKind::Activity).then_some(node)
    }

    pub fn validate_integrity(&self) -> bool {
        if self.nodes.len() < 2 {
            return false;
        }
        self.specification_version == SpecificationVersion::V0_1
            && self
                .nodes
                .first()
                .is_some_and(|node| node.kind == NodeKind::Start)
            && self
                .nodes
                .last()
                .is_some_and(|node| node.kind == NodeKind::Finish)
            && self.nodes[1..self.nodes.len() - 1]
                .iter()
                .all(|node| node.kind == NodeKind::Activity)
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
    let canonical = (
        SpecificationVersion::V0_1,
        workflow_id,
        nodes
            .iter()
            .map(|node| (&node.id, node.kind))
            .collect::<Vec<_>>(),
    );
    let bytes = serde_json::to_vec(&canonical).expect("canonical plan serialization cannot fail");
    let digest = Sha256::digest(bytes);
    PlanFingerprint::from_sha256(format!("{digest:x}")).expect("SHA-256 is valid hexadecimal")
}
