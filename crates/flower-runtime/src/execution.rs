use flower_ir::{NodeId, NodeKind, Workflow, WorkflowId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionStatus {
    Running,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionSnapshot {
    workflow_id: WorkflowId,
    status: ExecutionStatus,
    pending_node_id: Option<NodeId>,
    current_value: String,
    completed_node_ids: Vec<NodeId>,
}

impl ExecutionSnapshot {
    pub fn restore(
        workflow_id: WorkflowId,
        status: ExecutionStatus,
        pending_node_id: Option<NodeId>,
        current_value: String,
        completed_node_ids: Vec<NodeId>,
    ) -> Self {
        Self {
            workflow_id,
            status,
            pending_node_id,
            current_value,
            completed_node_ids,
        }
    }

    pub fn workflow_id(&self) -> &WorkflowId {
        &self.workflow_id
    }

    pub fn status(&self) -> ExecutionStatus {
        self.status
    }

    pub fn pending_node_id(&self) -> Option<&NodeId> {
        self.pending_node_id.as_ref()
    }

    pub fn current_value(&self) -> &str {
        &self.current_value
    }

    pub fn completed_node_ids(&self) -> &[NodeId] {
        &self.completed_node_ids
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionEvent {
    NodeCompleted { node_id: NodeId, output: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ExecutionEffect {
    ExecuteNode { node_id: NodeId, input: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    snapshot: ExecutionSnapshot,
    effects: Vec<ExecutionEffect>,
}

impl Transition {
    pub fn snapshot(&self) -> &ExecutionSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> ExecutionSnapshot {
        self.snapshot
    }

    pub fn effects(&self) -> &[ExecutionEffect] {
        &self.effects
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TransitionError {
    #[error("execution snapshot belongs to workflow `{actual}`, not `{expected}`")]
    WorkflowMismatch {
        expected: WorkflowId,
        actual: WorkflowId,
    },
    #[error("completed execution cannot accept more events")]
    ExecutionAlreadyCompleted,
    #[error("node `{actual}` completed while runtime was waiting for `{expected}`")]
    UnexpectedNodeCompletion { expected: NodeId, actual: NodeId },
    #[error("runtime reached non-activity node `{node_id}` between start and finish")]
    InvalidExecutableNode { node_id: NodeId },
    #[error("execution snapshot path is invalid; expected `{expected}`, found `{actual:?}`")]
    InvalidSnapshotPath {
        expected: NodeId,
        actual: Option<NodeId>,
    },
    #[error("execution snapshot status `{actual:?}` is invalid at node `{node_id}`")]
    InvalidSnapshotStatus {
        node_id: NodeId,
        actual: ExecutionStatus,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkflowEngine;

impl WorkflowEngine {
    pub fn start(
        &self,
        workflow: &Workflow,
        input: impl Into<String>,
    ) -> Result<Transition, TransitionError> {
        let snapshot = ExecutionSnapshot {
            workflow_id: workflow.id().clone(),
            status: ExecutionStatus::Running,
            pending_node_id: None,
            current_value: input.into(),
            completed_node_ids: Vec::new(),
        };
        self.move_to_successor(workflow, snapshot, workflow.start_node_id())
    }

    pub fn apply(
        &self,
        workflow: &Workflow,
        mut snapshot: ExecutionSnapshot,
        event: ExecutionEvent,
    ) -> Result<Transition, TransitionError> {
        if snapshot.workflow_id != *workflow.id() {
            return Err(TransitionError::WorkflowMismatch {
                expected: workflow.id().clone(),
                actual: snapshot.workflow_id,
            });
        }
        self.validate_snapshot(workflow, &snapshot)?;
        if snapshot.status == ExecutionStatus::Completed {
            return Err(TransitionError::ExecutionAlreadyCompleted);
        }

        let ExecutionEvent::NodeCompleted { node_id, output } = event;
        let expected = snapshot
            .pending_node_id
            .take()
            .expect("running executions always wait for a node");
        if node_id != expected {
            return Err(TransitionError::UnexpectedNodeCompletion {
                expected,
                actual: node_id,
            });
        }
        snapshot.completed_node_ids.push(node_id.clone());
        snapshot.current_value = output;
        self.move_to_successor(workflow, snapshot, &node_id)
    }

    fn validate_snapshot(
        &self,
        workflow: &Workflow,
        snapshot: &ExecutionSnapshot,
    ) -> Result<(), TransitionError> {
        let mut current_node_id = workflow.start_node_id();
        for completed_node_id in &snapshot.completed_node_ids {
            let expected_node_id = workflow
                .successor_of(current_node_id)
                .expect("completed path never advances beyond finish");
            let expected_node = workflow
                .node(expected_node_id)
                .expect("validated edge endpoint exists");
            if expected_node.kind != NodeKind::Activity || completed_node_id != expected_node_id {
                return Err(TransitionError::InvalidSnapshotPath {
                    expected: expected_node_id.clone(),
                    actual: Some(completed_node_id.clone()),
                });
            }
            current_node_id = completed_node_id;
        }

        let expected_node_id = workflow
            .successor_of(current_node_id)
            .expect("validated execution path ends at finish");
        match workflow
            .node(expected_node_id)
            .expect("validated edge endpoint exists")
            .kind
        {
            NodeKind::Activity if snapshot.status == ExecutionStatus::Running => {
                if snapshot.pending_node_id.as_ref() != Some(expected_node_id) {
                    return Err(TransitionError::InvalidSnapshotPath {
                        expected: expected_node_id.clone(),
                        actual: snapshot.pending_node_id.clone(),
                    });
                }
            }
            NodeKind::Finish if snapshot.status == ExecutionStatus::Completed => {
                if snapshot.pending_node_id.is_some() {
                    return Err(TransitionError::InvalidSnapshotPath {
                        expected: expected_node_id.clone(),
                        actual: snapshot.pending_node_id.clone(),
                    });
                }
            }
            _ => {
                return Err(TransitionError::InvalidSnapshotStatus {
                    node_id: expected_node_id.clone(),
                    actual: snapshot.status,
                });
            }
        }
        Ok(())
    }

    fn move_to_successor(
        &self,
        workflow: &Workflow,
        mut snapshot: ExecutionSnapshot,
        current_node_id: &NodeId,
    ) -> Result<Transition, TransitionError> {
        let successor_id = workflow
            .successor_of(current_node_id)
            .expect("validated non-finish nodes always have a successor");
        let successor = workflow
            .node(successor_id)
            .expect("validated edges always reference existing nodes");

        match successor.kind {
            NodeKind::Activity => {
                snapshot.pending_node_id = Some(successor.id.clone());
                let effect = ExecutionEffect::ExecuteNode {
                    node_id: successor.id.clone(),
                    input: snapshot.current_value.clone(),
                };
                Ok(Transition {
                    snapshot,
                    effects: vec![effect],
                })
            }
            NodeKind::Finish => {
                snapshot.status = ExecutionStatus::Completed;
                snapshot.pending_node_id = None;
                Ok(Transition {
                    snapshot,
                    effects: Vec::new(),
                })
            }
            NodeKind::Start => Err(TransitionError::InvalidExecutableNode {
                node_id: successor.id.clone(),
            }),
        }
    }
}
