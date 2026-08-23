use flower_compiler::{
    Diagnostic as DomainDiagnostic, EdgeDefinition as DomainEdge, NodeDefinition as DomainNode,
    WorkflowDefinition as DomainDefinition, compile,
};
use flower_kernel::{
    ExecutionEffect as DomainEffect, ExecutionEvent as DomainEvent, ExecutionRevision,
    ExecutionSnapshot as DomainSnapshot, ExecutionState as DomainState, Payload as DomainPayload,
    Transition as DomainTransition, transition,
};
use flower_plan::{
    EdgeId, EffectId, EventId, ExecutableWorkflowPlan as DomainPlan, ExecutionId, NodeId,
    NodeKind as DomainNodeKind, PlanFingerprint, PlanReference as DomainPlanReference,
    SpecificationVersion as DomainVersion, WorkflowId,
};

wit_bindgen::generate!({ path: "../../wits/engine.wit", world: "engine" });

use exports::flower::engine::workflow_engine::{
    AwaitingNode, Diagnostic, EdgeDefinition, EngineError, ExecutableWorkflowPlan,
    ExecuteNodeEffect, ExecutionEffect, ExecutionEvent, ExecutionSnapshot, ExecutionStartedEvent,
    ExecutionState, Guest, NodeCompletedEvent, NodeDefinition, NodeKind, Payload, PlanNode,
    PlanReference, SpecificationVersion, TransitionResult, WorkflowDefinition,
};

struct FlowerWorkflowEngine;

impl Guest for FlowerWorkflowEngine {
    fn compile(definition: WorkflowDefinition) -> Result<ExecutableWorkflowPlan, Vec<Diagnostic>> {
        definition
            .try_into_domain()
            .and_then(compile)
            .map(ExecutableWorkflowPlan::from_domain)
            .map_err(|diagnostics| {
                diagnostics
                    .into_iter()
                    .map(Diagnostic::from_domain)
                    .collect()
            })
    }

    fn transition(
        plan: ExecutableWorkflowPlan,
        snapshot: Option<ExecutionSnapshot>,
        event: ExecutionEvent,
    ) -> Result<TransitionResult, EngineError> {
        let plan = plan.try_into_domain()?;
        let snapshot = snapshot
            .map(ExecutionSnapshot::try_into_domain)
            .transpose()?;
        let event = event.try_into_domain()?;
        transition(&plan, snapshot.as_ref(), event)
            .map(TransitionResult::from_domain)
            .map_err(|error| EngineError {
                code: error.code().to_owned(),
                message: error.to_string(),
            })
    }
}

impl WorkflowDefinition {
    fn try_into_domain(self) -> Result<DomainDefinition, Vec<DomainDiagnostic>> {
        let invalid = |code: &str, message: String| {
            vec![DomainDiagnostic {
                code: code.to_owned(),
                message,
                subject: None,
            }]
        };
        Ok(DomainDefinition {
            id: WorkflowId::new(self.id)
                .map_err(|error| invalid("invalid-workflow-id", error.to_string()))?,
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
        })
    }
}

impl NodeDefinition {
    fn try_into_domain(self) -> Result<DomainNode, Vec<DomainDiagnostic>> {
        Ok(DomainNode {
            id: NodeId::new(self.id).map_err(identifier_diagnostic)?,
            kind: self.kind.into_domain(),
        })
    }
}

impl EdgeDefinition {
    fn try_into_domain(self) -> Result<DomainEdge, Vec<DomainDiagnostic>> {
        Ok(DomainEdge {
            id: EdgeId::new(self.id).map_err(identifier_diagnostic)?,
            source: NodeId::new(self.source).map_err(identifier_diagnostic)?,
            target: NodeId::new(self.target).map_err(identifier_diagnostic)?,
        })
    }
}

impl NodeKind {
    fn into_domain(self) -> DomainNodeKind {
        match self {
            Self::Start => DomainNodeKind::Start,
            Self::Activity => DomainNodeKind::Activity,
            Self::Finish => DomainNodeKind::Finish,
        }
    }
    fn from_domain(value: DomainNodeKind) -> Self {
        match value {
            DomainNodeKind::Start => Self::Start,
            DomainNodeKind::Activity => Self::Activity,
            DomainNodeKind::Finish => Self::Finish,
        }
    }
}

impl ExecutableWorkflowPlan {
    fn from_domain(plan: DomainPlan) -> Self {
        Self {
            specification_version: SpecificationVersion::from_domain(plan.specification_version()),
            workflow_id: plan.workflow_id().to_string(),
            fingerprint: plan.fingerprint().to_string(),
            nodes: plan
                .nodes()
                .iter()
                .map(|node| PlanNode {
                    id: node.id.to_string(),
                    kind: NodeKind::from_domain(node.kind),
                })
                .collect(),
        }
    }

    fn try_into_domain(self) -> Result<DomainPlan, EngineError> {
        if self.specification_version.try_into_domain()? != DomainVersion::V0_1 {
            return Err(engine_error(
                "unsupported-specification-version",
                "unsupported plan specification version",
            ));
        }
        let supplied_fingerprint = PlanFingerprint::from_sha256(self.fingerprint)
            .map_err(|error| engine_error("invalid-plan-fingerprint", error))?;
        let workflow_id = WorkflowId::new(self.workflow_id)
            .map_err(|error| engine_error("invalid-workflow-id", error))?;
        let nodes = self
            .nodes
            .into_iter()
            .map(|node| {
                Ok(DomainNode {
                    id: NodeId::new(node.id)
                        .map_err(|error| engine_error("invalid-node-id", error))?,
                    kind: node.kind.into_domain(),
                })
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let edges = nodes
            .windows(2)
            .enumerate()
            .map(|(index, pair)| DomainEdge {
                id: EdgeId::new(format!("normalized-edge-{index}"))
                    .expect("generated edge id is valid"),
                source: pair[0].id.clone(),
                target: pair[1].id.clone(),
            })
            .collect();
        let plan = compile(DomainDefinition {
            id: workflow_id,
            nodes,
            edges,
        })
        .map_err(|diagnostics| {
            engine_error(
                "invalid-plan",
                diagnostics
                    .into_iter()
                    .map(|value| value.message)
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        if plan.fingerprint() != &supplied_fingerprint {
            return Err(engine_error(
                "plan-fingerprint-mismatch",
                "serialized plan fingerprint does not match its contents",
            ));
        }
        Ok(plan)
    }
}

impl Diagnostic {
    fn from_domain(value: DomainDiagnostic) -> Self {
        Self {
            code: value.code,
            message: value.message,
            subject: value.subject,
        }
    }
}
impl SpecificationVersion {
    fn from_domain(value: DomainVersion) -> Self {
        Self {
            major: value.major,
            minor: value.minor,
        }
    }
    fn try_into_domain(self) -> Result<DomainVersion, EngineError> {
        Ok(DomainVersion {
            major: self.major,
            minor: self.minor,
        })
    }
}
impl Payload {
    fn from_domain(value: DomainPayload) -> Self {
        Self {
            media_type: value.media_type,
            bytes: value.bytes,
        }
    }
    fn into_domain(self) -> DomainPayload {
        DomainPayload {
            media_type: self.media_type,
            bytes: self.bytes,
        }
    }
}

impl ExecutionSnapshot {
    fn try_into_domain(self) -> Result<DomainSnapshot, EngineError> {
        Ok(DomainSnapshot {
            execution_id: ExecutionId::new(self.execution_id)
                .map_err(|error| engine_error("invalid-execution-id", error))?,
            plan_reference: self.plan_reference.try_into_domain()?,
            revision: ExecutionRevision(self.revision),
            state: self.state.try_into_domain()?,
        })
    }
    fn from_domain(value: DomainSnapshot) -> Self {
        Self {
            execution_id: value.execution_id.to_string(),
            plan_reference: PlanReference::from_domain(value.plan_reference),
            revision: value.revision.0,
            state: ExecutionState::from_domain(value.state),
        }
    }
}

impl PlanReference {
    fn try_into_domain(self) -> Result<DomainPlanReference, EngineError> {
        Ok(DomainPlanReference {
            specification_version: self.specification_version.try_into_domain()?,
            workflow_id: WorkflowId::new(self.workflow_id)
                .map_err(|error| engine_error("invalid-workflow-id", error))?,
            fingerprint: PlanFingerprint::from_sha256(self.fingerprint)
                .map_err(|error| engine_error("invalid-plan-fingerprint", error))?,
        })
    }

    fn from_domain(value: DomainPlanReference) -> Self {
        Self {
            specification_version: SpecificationVersion::from_domain(value.specification_version),
            workflow_id: value.workflow_id.to_string(),
            fingerprint: value.fingerprint.to_string(),
        }
    }
}

impl ExecutionState {
    fn try_into_domain(self) -> Result<DomainState, EngineError> {
        match self {
            Self::AwaitingNode(value) => Ok(DomainState::AwaitingNode {
                node_id: NodeId::new(value.node_id)
                    .map_err(|error| engine_error("invalid-node-id", error))?,
                effect_id: EffectId::new(value.effect_id)
                    .map_err(|error| engine_error("invalid-effect-id", error))?,
                input: value.input.into_domain(),
            }),
            Self::Completed(value) => Ok(DomainState::Completed {
                output: value.into_domain(),
            }),
        }
    }
    fn from_domain(value: DomainState) -> Self {
        match value {
            DomainState::AwaitingNode {
                node_id,
                effect_id,
                input,
            } => Self::AwaitingNode(AwaitingNode {
                node_id: node_id.to_string(),
                effect_id: effect_id.to_string(),
                input: Payload::from_domain(input),
            }),
            DomainState::Completed { output } => Self::Completed(Payload::from_domain(output)),
        }
    }
}

impl ExecutionEvent {
    fn try_into_domain(self) -> Result<DomainEvent, EngineError> {
        match self {
            Self::ExecutionStarted(ExecutionStartedEvent {
                event_id,
                execution_id,
                plan_reference,
                input,
            }) => Ok(DomainEvent::ExecutionStarted {
                event_id: EventId::new(event_id)
                    .map_err(|error| engine_error("invalid-event-id", error))?,
                execution_id: ExecutionId::new(execution_id)
                    .map_err(|error| engine_error("invalid-execution-id", error))?,
                plan_reference: plan_reference.try_into_domain()?,
                input: input.into_domain(),
            }),
            Self::NodeCompleted(NodeCompletedEvent {
                event_id,
                execution_id,
                expected_revision,
                effect_id,
                node_id,
                output,
            }) => Ok(DomainEvent::NodeCompleted {
                event_id: EventId::new(event_id)
                    .map_err(|error| engine_error("invalid-event-id", error))?,
                execution_id: ExecutionId::new(execution_id)
                    .map_err(|error| engine_error("invalid-execution-id", error))?,
                expected_revision: ExecutionRevision(expected_revision),
                effect_id: EffectId::new(effect_id)
                    .map_err(|error| engine_error("invalid-effect-id", error))?,
                node_id: NodeId::new(node_id)
                    .map_err(|error| engine_error("invalid-node-id", error))?,
                output: output.into_domain(),
            }),
        }
    }
}

impl TransitionResult {
    fn from_domain(value: DomainTransition) -> Self {
        Self {
            snapshot: ExecutionSnapshot::from_domain(value.snapshot),
            effects: value
                .effects
                .into_iter()
                .map(ExecutionEffect::from_domain)
                .collect(),
        }
    }
}
impl ExecutionEffect {
    fn from_domain(value: DomainEffect) -> Self {
        match value {
            DomainEffect::ExecuteNode {
                effect_id,
                execution_id,
                node_id,
                input,
            } => Self::ExecuteNode(ExecuteNodeEffect {
                effect_id: effect_id.to_string(),
                execution_id: execution_id.to_string(),
                node_id: node_id.to_string(),
                input: Payload::from_domain(input),
            }),
        }
    }
}

fn identifier_diagnostic(error: impl std::fmt::Display) -> Vec<DomainDiagnostic> {
    vec![DomainDiagnostic {
        code: "invalid-identifier".to_owned(),
        message: error.to_string(),
        subject: None,
    }]
}
fn engine_error(code: &str, error: impl std::fmt::Display) -> EngineError {
    EngineError {
        code: code.to_owned(),
        message: error.to_string(),
    }
}

export!(FlowerWorkflowEngine with_types_in self);
