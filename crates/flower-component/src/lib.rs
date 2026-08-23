use flower_compiler::{
    Diagnostic as DomainDiagnostic, EdgeDefinition as DomainEdge, NodeDefinition as DomainNode,
    WorkflowDefinition as DomainDefinition, compile,
};
use flower_kernel::{
    AttemptFailure as DomainFailure, AttemptNumber as DomainAttemptNumber,
    ExecutionEffect as DomainEffect, ExecutionEvent as DomainEvent, ExecutionRevision,
    ExecutionSnapshot as DomainSnapshot, ExecutionState as DomainState,
    NodeActivation as DomainActivation, NodeAttempt as DomainAttempt, Payload as DomainPayload,
    RetryTimer as DomainRetryTimer, Transition as DomainTransition, transition,
};
use flower_plan::{
    AttemptId, BackoffPolicy as DomainBackoff, EdgeId, EffectId, EventId,
    ExecutableWorkflowPlan as DomainPlan, ExecutionId, FailureCode, NodeActivationId, NodeId,
    NodeKind as DomainNodeKind, PlanFingerprint, PlanReference as DomainPlanReference,
    RetryPolicy as DomainRetryPolicy, SpecificationVersion as DomainVersion, TimerId, WorkflowId,
};

wit_bindgen::generate!({ path: "../../wits/engine.wit", world: "engine" });

use exports::flower::engine::workflow_engine::{
    AttemptFailure, AwaitingAttempt, BackoffPolicy, Diagnostic, EdgeDefinition, EngineError,
    ExecutableWorkflowPlan, ExecuteNodeAttemptEffect, ExecutionEffect, ExecutionEvent,
    ExecutionSnapshot, ExecutionStartedEvent, ExecutionState, ExponentialBackoff, FailedExecution,
    FixedBackoff, Guest, NodeActivation, NodeAttempt, NodeAttemptFailedEvent,
    NodeAttemptSucceededEvent, NodeDefinition, NodeKind, Payload, PlanNode, PlanReference,
    RetryPolicy, RetryTimer, ScheduleTimerEffect, SpecificationVersion, TimerFiredEvent,
    TransitionResult, WaitingForRetry, WorkflowDefinition,
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
            retry_policy: self
                .retry_policy
                .map(RetryPolicy::try_into_domain)
                .transpose()
                .map_err(|error| {
                    vec![DomainDiagnostic {
                        code: "invalid-retry-policy".to_owned(),
                        message: error.to_string(),
                        subject: None,
                    }]
                })?,
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

impl RetryPolicy {
    fn try_into_domain(self) -> Result<DomainRetryPolicy, EngineError> {
        let retryable_failure_codes = self
            .retryable_failure_codes
            .into_iter()
            .map(|code| {
                FailureCode::new(code).map_err(|error| engine_error("invalid-failure-code", error))
            })
            .collect::<Result<_, _>>()?;
        let policy = DomainRetryPolicy {
            max_attempts: self.max_attempts,
            retryable_failure_codes,
            backoff: self.backoff.into_domain(),
        };
        policy.validate().then_some(policy).ok_or_else(|| {
            engine_error(
                "invalid-retry-policy",
                "retry policy violates its numeric invariants",
            )
        })
    }

    fn from_domain(value: DomainRetryPolicy) -> Self {
        Self {
            max_attempts: value.max_attempts,
            retryable_failure_codes: value
                .retryable_failure_codes
                .into_iter()
                .map(|code| code.to_string())
                .collect(),
            backoff: BackoffPolicy::from_domain(value.backoff),
        }
    }
}

impl BackoffPolicy {
    fn into_domain(self) -> DomainBackoff {
        match self {
            Self::None => DomainBackoff::None,
            Self::Fixed(FixedBackoff { delay_ms }) => DomainBackoff::Fixed { delay_ms },
            Self::Exponential(ExponentialBackoff {
                initial_delay_ms,
                multiplier,
                maximum_delay_ms,
            }) => DomainBackoff::Exponential {
                initial_delay_ms,
                multiplier,
                maximum_delay_ms,
            },
        }
    }

    fn from_domain(value: DomainBackoff) -> Self {
        match value {
            DomainBackoff::None => Self::None,
            DomainBackoff::Fixed { delay_ms } => Self::Fixed(FixedBackoff { delay_ms }),
            DomainBackoff::Exponential {
                initial_delay_ms,
                multiplier,
                maximum_delay_ms,
            } => Self::Exponential(ExponentialBackoff {
                initial_delay_ms,
                multiplier,
                maximum_delay_ms,
            }),
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
                    retry_policy: node.retry_policy.clone().map(RetryPolicy::from_domain),
                })
                .collect(),
        }
    }

    fn try_into_domain(self) -> Result<DomainPlan, EngineError> {
        if self.specification_version.try_into_domain()? != DomainVersion::V0_2 {
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
                    retry_policy: node
                        .retry_policy
                        .map(RetryPolicy::try_into_domain)
                        .transpose()?,
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
            Self::AwaitingAttempt(value) => Ok(DomainState::AwaitingAttempt {
                activation: value.activation.try_into_domain()?,
                attempt: value.attempt.try_into_domain()?,
            }),
            Self::WaitingForRetry(value) => Ok(DomainState::WaitingForRetry {
                activation: value.activation.try_into_domain()?,
                attempt: value.attempt.try_into_domain()?,
                failure: value.failure.try_into_domain()?,
                timer: value.timer.try_into_domain()?,
            }),
            Self::Completed(value) => Ok(DomainState::Completed {
                output: value.into_domain(),
            }),
            Self::Failed(value) => Ok(DomainState::Failed {
                activation: value.activation.try_into_domain()?,
                attempt: value.attempt.try_into_domain()?,
                failure: value.failure.try_into_domain()?,
            }),
        }
    }
    fn from_domain(value: DomainState) -> Self {
        match value {
            DomainState::AwaitingAttempt {
                activation,
                attempt,
            } => Self::AwaitingAttempt(AwaitingAttempt {
                activation: NodeActivation::from_domain(activation),
                attempt: NodeAttempt::from_domain(attempt),
            }),
            DomainState::WaitingForRetry {
                activation,
                attempt,
                failure,
                timer,
            } => Self::WaitingForRetry(WaitingForRetry {
                activation: NodeActivation::from_domain(activation),
                attempt: NodeAttempt::from_domain(attempt),
                failure: AttemptFailure::from_domain(failure),
                timer: RetryTimer::from_domain(timer),
            }),
            DomainState::Completed { output } => Self::Completed(Payload::from_domain(output)),
            DomainState::Failed {
                activation,
                attempt,
                failure,
            } => Self::Failed(FailedExecution {
                activation: NodeActivation::from_domain(activation),
                attempt: NodeAttempt::from_domain(attempt),
                failure: AttemptFailure::from_domain(failure),
            }),
        }
    }
}

impl NodeActivation {
    fn try_into_domain(self) -> Result<DomainActivation, EngineError> {
        Ok(DomainActivation {
            activation_id: NodeActivationId::new(self.activation_id)
                .map_err(|error| engine_error("invalid-activation-id", error))?,
            activation_revision: ExecutionRevision(self.activation_revision),
            node_id: NodeId::new(self.node_id)
                .map_err(|error| engine_error("invalid-node-id", error))?,
            input: self.input.into_domain(),
        })
    }
    fn from_domain(value: DomainActivation) -> Self {
        Self {
            activation_id: value.activation_id.to_string(),
            activation_revision: value.activation_revision.0,
            node_id: value.node_id.to_string(),
            input: Payload::from_domain(value.input),
        }
    }
}

impl NodeAttempt {
    fn try_into_domain(self) -> Result<DomainAttempt, EngineError> {
        Ok(DomainAttempt {
            attempt_id: AttemptId::new(self.attempt_id)
                .map_err(|error| engine_error("invalid-attempt-id", error))?,
            attempt_number: parse_attempt_number(self.attempt_number)?,
            effect_id: EffectId::new(self.effect_id)
                .map_err(|error| engine_error("invalid-effect-id", error))?,
        })
    }
    fn from_domain(value: DomainAttempt) -> Self {
        Self {
            attempt_id: value.attempt_id.to_string(),
            attempt_number: value.attempt_number.value(),
            effect_id: value.effect_id.to_string(),
        }
    }
}

impl AttemptFailure {
    fn try_into_domain(self) -> Result<DomainFailure, EngineError> {
        Ok(DomainFailure {
            code: FailureCode::new(self.code)
                .map_err(|error| engine_error("invalid-failure-code", error))?,
            details: self.details.map(Payload::into_domain),
        })
    }
    fn from_domain(value: DomainFailure) -> Self {
        Self {
            code: value.code.to_string(),
            details: value.details.map(Payload::from_domain),
        }
    }
}

impl RetryTimer {
    fn try_into_domain(self) -> Result<DomainRetryTimer, EngineError> {
        Ok(DomainRetryTimer {
            timer_id: TimerId::new(self.timer_id)
                .map_err(|error| engine_error("invalid-timer-id", error))?,
            effect_id: EffectId::new(self.effect_id)
                .map_err(|error| engine_error("invalid-effect-id", error))?,
            failed_attempt_id: AttemptId::new(self.failed_attempt_id)
                .map_err(|error| engine_error("invalid-attempt-id", error))?,
            next_attempt_number: parse_attempt_number(self.next_attempt_number)?,
            delay_ms: self.delay_ms,
        })
    }

    fn from_domain(value: DomainRetryTimer) -> Self {
        Self {
            timer_id: value.timer_id.to_string(),
            effect_id: value.effect_id.to_string(),
            failed_attempt_id: value.failed_attempt_id.to_string(),
            next_attempt_number: value.next_attempt_number.value(),
            delay_ms: value.delay_ms,
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
            Self::NodeAttemptSucceeded(NodeAttemptSucceededEvent {
                event_id,
                execution_id,
                expected_revision,
                activation_id,
                attempt_id,
                attempt_number,
                effect_id,
                node_id,
                output,
            }) => Ok(DomainEvent::NodeAttemptSucceeded {
                event_id: EventId::new(event_id)
                    .map_err(|error| engine_error("invalid-event-id", error))?,
                execution_id: ExecutionId::new(execution_id)
                    .map_err(|error| engine_error("invalid-execution-id", error))?,
                expected_revision: ExecutionRevision(expected_revision),
                activation_id: NodeActivationId::new(activation_id)
                    .map_err(|error| engine_error("invalid-activation-id", error))?,
                attempt_id: AttemptId::new(attempt_id)
                    .map_err(|error| engine_error("invalid-attempt-id", error))?,
                attempt_number: parse_attempt_number(attempt_number)?,
                effect_id: EffectId::new(effect_id)
                    .map_err(|error| engine_error("invalid-effect-id", error))?,
                node_id: NodeId::new(node_id)
                    .map_err(|error| engine_error("invalid-node-id", error))?,
                output: output.into_domain(),
            }),
            Self::NodeAttemptFailed(NodeAttemptFailedEvent {
                event_id,
                execution_id,
                expected_revision,
                activation_id,
                attempt_id,
                attempt_number,
                effect_id,
                node_id,
                failure,
            }) => Ok(DomainEvent::NodeAttemptFailed {
                event_id: EventId::new(event_id)
                    .map_err(|error| engine_error("invalid-event-id", error))?,
                execution_id: ExecutionId::new(execution_id)
                    .map_err(|error| engine_error("invalid-execution-id", error))?,
                expected_revision: ExecutionRevision(expected_revision),
                activation_id: NodeActivationId::new(activation_id)
                    .map_err(|error| engine_error("invalid-activation-id", error))?,
                attempt_id: AttemptId::new(attempt_id)
                    .map_err(|error| engine_error("invalid-attempt-id", error))?,
                attempt_number: parse_attempt_number(attempt_number)?,
                effect_id: EffectId::new(effect_id)
                    .map_err(|error| engine_error("invalid-effect-id", error))?,
                node_id: NodeId::new(node_id)
                    .map_err(|error| engine_error("invalid-node-id", error))?,
                failure: failure.try_into_domain()?,
            }),
            Self::TimerFired(TimerFiredEvent {
                event_id,
                execution_id,
                expected_revision,
                timer_id,
                activation_id,
                next_attempt_number,
            }) => Ok(DomainEvent::TimerFired {
                event_id: EventId::new(event_id)
                    .map_err(|error| engine_error("invalid-event-id", error))?,
                execution_id: ExecutionId::new(execution_id)
                    .map_err(|error| engine_error("invalid-execution-id", error))?,
                expected_revision: ExecutionRevision(expected_revision),
                timer_id: TimerId::new(timer_id)
                    .map_err(|error| engine_error("invalid-timer-id", error))?,
                activation_id: NodeActivationId::new(activation_id)
                    .map_err(|error| engine_error("invalid-activation-id", error))?,
                next_attempt_number: parse_attempt_number(next_attempt_number)?,
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
            DomainEffect::ExecuteNodeAttempt {
                effect_id,
                activation_id,
                attempt_id,
                attempt_number,
                node_id,
                input,
            } => Self::ExecuteNodeAttempt(ExecuteNodeAttemptEffect {
                effect_id: effect_id.to_string(),
                activation_id: activation_id.to_string(),
                attempt_id: attempt_id.to_string(),
                attempt_number: attempt_number.value(),
                node_id: node_id.to_string(),
                input: Payload::from_domain(input),
            }),
            DomainEffect::ScheduleTimer {
                effect_id,
                timer_id,
                activation_id,
                failed_attempt_id,
                next_attempt_number,
                delay_ms,
            } => Self::ScheduleTimer(ScheduleTimerEffect {
                effect_id: effect_id.to_string(),
                timer_id: timer_id.to_string(),
                activation_id: activation_id.to_string(),
                failed_attempt_id: failed_attempt_id.to_string(),
                next_attempt_number: next_attempt_number.value(),
                delay_ms,
            }),
        }
    }
}

fn parse_attempt_number(value: u32) -> Result<DomainAttemptNumber, EngineError> {
    DomainAttemptNumber::new(value)
        .ok_or_else(|| engine_error("invalid-attempt-number", "attempt number must be non-zero"))
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
