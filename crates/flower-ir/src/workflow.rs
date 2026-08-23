use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{EdgeId, NodeId, WorkflowId};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeKind {
    Start,
    Activity,
    Finish,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NodeDefinition {
    pub id: NodeId,
    pub kind: NodeKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EdgeDefinition {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkflowDefinition {
    pub id: WorkflowId,
    pub nodes: Vec<NodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum WorkflowValidationError {
    #[error("workflow contains duplicate node id `{node_id}`")]
    DuplicateNode { node_id: NodeId },
    #[error("workflow contains duplicate edge id `{edge_id}`")]
    DuplicateEdge { edge_id: EdgeId },
    #[error("edge `{edge_id}` references missing node `{node_id}`")]
    MissingEdgeEndpoint { edge_id: EdgeId, node_id: NodeId },
    #[error("workflow must contain exactly one start node; found {actual}")]
    InvalidStartNodeCount { actual: usize },
    #[error("workflow must contain exactly one finish node; found {actual}")]
    InvalidFinishNodeCount { actual: usize },
    #[error("start node `{node_id}` must not have incoming edges")]
    StartHasIncomingEdge { node_id: NodeId },
    #[error("finish node `{node_id}` must not have outgoing edges")]
    FinishHasOutgoingEdge { node_id: NodeId },
    #[error("node `{node_id}` has no incoming edge")]
    MissingIncomingEdge { node_id: NodeId },
    #[error("node `{node_id}` has no outgoing edge")]
    MissingOutgoingEdge { node_id: NodeId },
    #[error("node `{node_id}` has multiple incoming edges; gateways are not implemented")]
    MultipleIncomingEdgesUnsupported { node_id: NodeId },
    #[error("node `{node_id}` has multiple outgoing edges; gateways are not implemented")]
    MultipleOutgoingEdgesUnsupported { node_id: NodeId },
    #[error("workflow contains a cycle at node `{node_id}`")]
    Cycle { node_id: NodeId },
    #[error("node `{node_id}` is disconnected from the start-to-finish path")]
    DisconnectedNode { node_id: NodeId },
}

/// A validated, normalized workflow aggregate that is safe for the runtime to execute.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workflow {
    id: WorkflowId,
    nodes: BTreeMap<NodeId, NodeDefinition>,
    start_node_id: NodeId,
    finish_node_id: NodeId,
    successor_by_node_id: BTreeMap<NodeId, NodeId>,
}

impl Workflow {
    pub fn id(&self) -> &WorkflowId {
        &self.id
    }

    pub fn start_node_id(&self) -> &NodeId {
        &self.start_node_id
    }

    pub fn finish_node_id(&self) -> &NodeId {
        &self.finish_node_id
    }

    pub fn node(&self, node_id: &NodeId) -> Option<&NodeDefinition> {
        self.nodes.get(node_id)
    }

    pub fn successor_of(&self, node_id: &NodeId) -> Option<&NodeId> {
        self.successor_by_node_id.get(node_id)
    }
}

impl TryFrom<WorkflowDefinition> for Workflow {
    type Error = WorkflowValidationError;

    fn try_from(definition: WorkflowDefinition) -> Result<Self, Self::Error> {
        let mut nodes = BTreeMap::new();
        for node in definition.nodes {
            let node_id = node.id.clone();
            if nodes.insert(node_id.clone(), node).is_some() {
                return Err(WorkflowValidationError::DuplicateNode { node_id });
            }
        }

        let start_node_id = exactly_one_node_of_kind(&nodes, NodeKind::Start)?;
        let finish_node_id = exactly_one_node_of_kind(&nodes, NodeKind::Finish)?;
        let mut edge_ids = BTreeSet::new();
        let mut incoming_counts = BTreeMap::<NodeId, usize>::new();
        let mut outgoing_counts = BTreeMap::<NodeId, usize>::new();
        let mut successor_by_node_id = BTreeMap::new();

        for edge in definition.edges {
            if !edge_ids.insert(edge.id.clone()) {
                return Err(WorkflowValidationError::DuplicateEdge { edge_id: edge.id });
            }
            for endpoint in [&edge.source, &edge.target] {
                if !nodes.contains_key(endpoint) {
                    return Err(WorkflowValidationError::MissingEdgeEndpoint {
                        edge_id: edge.id,
                        node_id: endpoint.clone(),
                    });
                }
            }

            *incoming_counts.entry(edge.target.clone()).or_default() += 1;
            *outgoing_counts.entry(edge.source.clone()).or_default() += 1;
            successor_by_node_id.insert(edge.source, edge.target);
        }

        validate_degrees(
            &nodes,
            &incoming_counts,
            &outgoing_counts,
            &start_node_id,
            &finish_node_id,
        )?;
        validate_single_path(
            &nodes,
            &successor_by_node_id,
            &start_node_id,
            &finish_node_id,
        )?;

        Ok(Self {
            id: definition.id,
            nodes,
            start_node_id,
            finish_node_id,
            successor_by_node_id,
        })
    }
}

fn exactly_one_node_of_kind(
    nodes: &BTreeMap<NodeId, NodeDefinition>,
    expected_kind: NodeKind,
) -> Result<NodeId, WorkflowValidationError> {
    let matching_ids = nodes
        .values()
        .filter(|node| node.kind == expected_kind)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    if matching_ids.len() == 1 {
        return Ok(matching_ids[0].clone());
    }
    match expected_kind {
        NodeKind::Start => Err(WorkflowValidationError::InvalidStartNodeCount {
            actual: matching_ids.len(),
        }),
        NodeKind::Finish => Err(WorkflowValidationError::InvalidFinishNodeCount {
            actual: matching_ids.len(),
        }),
        NodeKind::Activity => unreachable!("activity nodes are not unique control nodes"),
    }
}

fn validate_degrees(
    nodes: &BTreeMap<NodeId, NodeDefinition>,
    incoming_counts: &BTreeMap<NodeId, usize>,
    outgoing_counts: &BTreeMap<NodeId, usize>,
    start_node_id: &NodeId,
    finish_node_id: &NodeId,
) -> Result<(), WorkflowValidationError> {
    let nodes_in_diagnostic_order = std::iter::once(
        nodes
            .get(start_node_id)
            .expect("start node was selected from the node map"),
    )
    .chain(std::iter::once(
        nodes
            .get(finish_node_id)
            .expect("finish node was selected from the node map"),
    ))
    .chain(
        nodes
            .values()
            .filter(|node| &node.id != start_node_id && &node.id != finish_node_id),
    );

    for node in nodes_in_diagnostic_order {
        let incoming = incoming_counts.get(&node.id).copied().unwrap_or_default();
        let outgoing = outgoing_counts.get(&node.id).copied().unwrap_or_default();

        if &node.id == start_node_id && incoming != 0 {
            return Err(WorkflowValidationError::StartHasIncomingEdge {
                node_id: node.id.clone(),
            });
        }
        if &node.id == finish_node_id && outgoing != 0 {
            return Err(WorkflowValidationError::FinishHasOutgoingEdge {
                node_id: node.id.clone(),
            });
        }
        if &node.id != start_node_id {
            match incoming {
                0 => {
                    return Err(WorkflowValidationError::MissingIncomingEdge {
                        node_id: node.id.clone(),
                    });
                }
                1 => {}
                _ => {
                    return Err(WorkflowValidationError::MultipleIncomingEdgesUnsupported {
                        node_id: node.id.clone(),
                    });
                }
            }
        }
        if &node.id != finish_node_id {
            match outgoing {
                0 => {
                    return Err(WorkflowValidationError::MissingOutgoingEdge {
                        node_id: node.id.clone(),
                    });
                }
                1 => {}
                _ => {
                    return Err(WorkflowValidationError::MultipleOutgoingEdgesUnsupported {
                        node_id: node.id.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn validate_single_path(
    nodes: &BTreeMap<NodeId, NodeDefinition>,
    successors: &BTreeMap<NodeId, NodeId>,
    start_node_id: &NodeId,
    finish_node_id: &NodeId,
) -> Result<(), WorkflowValidationError> {
    let mut visited = BTreeSet::new();
    let mut current = start_node_id;
    loop {
        if !visited.insert(current.clone()) {
            return Err(WorkflowValidationError::Cycle {
                node_id: current.clone(),
            });
        }
        if current == finish_node_id {
            break;
        }
        current = successors
            .get(current)
            .expect("degree validation guarantees a successor");
    }

    if let Some(disconnected_id) = nodes.keys().find(|node_id| !visited.contains(*node_id)) {
        return Err(WorkflowValidationError::DisconnectedNode {
            node_id: disconnected_id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: NodeKind) -> NodeDefinition {
        NodeDefinition {
            id: NodeId::new(id).unwrap(),
            kind,
        }
    }

    fn edge(id: &str, source: &str, target: &str) -> EdgeDefinition {
        EdgeDefinition {
            id: EdgeId::new(id).unwrap(),
            source: NodeId::new(source).unwrap(),
            target: NodeId::new(target).unwrap(),
        }
    }

    #[test]
    fn normalizes_a_linear_workflow() {
        let workflow = Workflow::try_from(WorkflowDefinition {
            id: WorkflowId::new("greeting").unwrap(),
            nodes: vec![
                node("start", NodeKind::Start),
                node("node1", NodeKind::Activity),
                node("node2", NodeKind::Activity),
                node("finish", NodeKind::Finish),
            ],
            edges: vec![
                edge("e1", "start", "node1"),
                edge("e2", "node1", "node2"),
                edge("e3", "node2", "finish"),
            ],
        })
        .unwrap();

        assert_eq!(
            workflow.successor_of(&NodeId::new("node1").unwrap()),
            Some(&NodeId::new("node2").unwrap())
        );
    }

    #[test]
    fn rejects_branching_until_gateway_semantics_exist() {
        let error = Workflow::try_from(WorkflowDefinition {
            id: WorkflowId::new("branching").unwrap(),
            nodes: vec![
                node("start", NodeKind::Start),
                node("left", NodeKind::Activity),
                node("right", NodeKind::Activity),
                node("finish", NodeKind::Finish),
            ],
            edges: vec![
                edge("e1", "start", "left"),
                edge("e2", "start", "right"),
                edge("e3", "left", "finish"),
                edge("e4", "right", "finish"),
            ],
        })
        .unwrap_err();

        assert_eq!(
            error,
            WorkflowValidationError::MultipleOutgoingEdgesUnsupported {
                node_id: NodeId::new("start").unwrap()
            }
        );
    }
}
