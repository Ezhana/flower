use flower_ir::{
    EdgeDefinition as DomainEdge, EdgeId, NodeDefinition as DomainNode, NodeId,
    NodeKind as DomainNodeKind, Workflow, WorkflowDefinition as DomainWorkflow, WorkflowId,
};
use flower_runtime::{
    ExecutionEffect as DomainEffect, ExecutionEvent, ExecutionSnapshot as DomainSnapshot,
    ExecutionStatus as DomainStatus, Transition as DomainTransition, WorkflowEngine,
};

wit_bindgen::generate!({
    path: "../../wits/engine.wit",
    world: "engine",
});

use exports::flower::engine::workflow_engine::{
    EdgeDefinition, EngineError, ExecuteNodeEffect, ExecutionSnapshot, ExecutionStatus, Guest,
    NodeDefinition, NodeKind, Transition, WorkflowDefinition,
};

struct FlowerWorkflowEngine;

impl Guest for FlowerWorkflowEngine {
    fn start(workflow: WorkflowDefinition, input: String) -> Result<Transition, EngineError> {
        let workflow = workflow.try_into_domain()?;
        WorkflowEngine
            .start(&workflow, input)
            .map(Transition::from_domain)
            .map_err(|error| engine_error("transition-failed", error))
    }

    fn complete_node(
        workflow: WorkflowDefinition,
        snapshot: ExecutionSnapshot,
        node_id: String,
        output: String,
    ) -> Result<Transition, EngineError> {
        let workflow = workflow.try_into_domain()?;
        let snapshot = snapshot.try_into_domain()?;
        let node_id = parse_node_id(node_id)?;
        WorkflowEngine
            .apply(
                &workflow,
                snapshot,
                ExecutionEvent::NodeCompleted { node_id, output },
            )
            .map(Transition::from_domain)
            .map_err(|error| engine_error("transition-failed", error))
    }
}

impl WorkflowDefinition {
    fn try_into_domain(self) -> Result<Workflow, EngineError> {
        let definition = DomainWorkflow {
            id: WorkflowId::new(self.id)
                .map_err(|error| engine_error("invalid-workflow-id", error))?,
            nodes: self
                .nodes
                .into_iter()
                .map(NodeDefinition::try_into_domain)
                .collect::<Result<_, _>>()?,
            edges: self
                .edges
                .into_iter()
                .map(EdgeDefinition::try_into_domain)
                .collect::<Result<_, _>>()?,
        };
        Workflow::try_from(definition).map_err(|error| engine_error("invalid-workflow", error))
    }
}

impl NodeDefinition {
    fn try_into_domain(self) -> Result<DomainNode, EngineError> {
        Ok(DomainNode {
            id: parse_node_id(self.id)?,
            kind: match self.kind {
                NodeKind::Start => DomainNodeKind::Start,
                NodeKind::Activity => DomainNodeKind::Activity,
                NodeKind::Finish => DomainNodeKind::Finish,
            },
        })
    }
}

impl EdgeDefinition {
    fn try_into_domain(self) -> Result<DomainEdge, EngineError> {
        Ok(DomainEdge {
            id: EdgeId::new(self.id).map_err(|error| engine_error("invalid-edge-id", error))?,
            source: parse_node_id(self.source)?,
            target: parse_node_id(self.target)?,
        })
    }
}

impl ExecutionSnapshot {
    fn try_into_domain(self) -> Result<DomainSnapshot, EngineError> {
        Ok(DomainSnapshot::restore(
            WorkflowId::new(self.workflow_id)
                .map_err(|error| engine_error("invalid-workflow-id", error))?,
            match self.status {
                ExecutionStatus::Running => DomainStatus::Running,
                ExecutionStatus::Completed => DomainStatus::Completed,
            },
            self.pending_node_id.map(parse_node_id).transpose()?,
            self.current_value,
            self.completed_node_ids
                .into_iter()
                .map(parse_node_id)
                .collect::<Result<_, _>>()?,
        ))
    }

    fn from_domain(snapshot: &DomainSnapshot) -> Self {
        Self {
            workflow_id: snapshot.workflow_id().to_string(),
            status: match snapshot.status() {
                DomainStatus::Running => ExecutionStatus::Running,
                DomainStatus::Completed => ExecutionStatus::Completed,
            },
            pending_node_id: snapshot.pending_node_id().map(ToString::to_string),
            current_value: snapshot.current_value().to_owned(),
            completed_node_ids: snapshot
                .completed_node_ids()
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

impl Transition {
    fn from_domain(transition: DomainTransition) -> Self {
        let effects = transition
            .effects()
            .iter()
            .map(|effect| match effect {
                DomainEffect::ExecuteNode { node_id, input } => ExecuteNodeEffect {
                    node_id: node_id.to_string(),
                    input: input.clone(),
                },
            })
            .collect();
        Self {
            snapshot: ExecutionSnapshot::from_domain(transition.snapshot()),
            effects,
        }
    }
}

fn parse_node_id(value: String) -> Result<NodeId, EngineError> {
    NodeId::new(value).map_err(|error| engine_error("invalid-node-id", error))
}

fn engine_error(code: &str, error: impl std::fmt::Display) -> EngineError {
    EngineError {
        code: code.to_owned(),
        message: error.to_string(),
    }
}

export!(FlowerWorkflowEngine with_types_in self);
