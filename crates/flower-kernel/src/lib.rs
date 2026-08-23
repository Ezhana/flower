use flower_plan::{
    AttemptId, EffectId, EventId, ExecutableWorkflowPlan, ExecutionId, FailureCode,
    NodeActivationId, NodeId, NodeKind, PlanReference, TimerId,
};
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub media_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ExecutionRevision(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AttemptNumber(NonZeroU32);

impl AttemptNumber {
    pub const FIRST: Self = Self(NonZeroU32::MIN);

    pub fn new(value: u32) -> Option<Self> {
        NonZeroU32::new(value).map(Self)
    }

    pub fn value(self) -> u32 {
        self.0.get()
    }

    pub fn checked_next(self) -> Option<Self> {
        self.value().checked_add(1).and_then(Self::new)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeActivation {
    pub activation_id: NodeActivationId,
    pub activation_revision: ExecutionRevision,
    pub node_id: NodeId,
    pub input: Payload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAttempt {
    pub attempt_id: AttemptId,
    pub attempt_number: AttemptNumber,
    pub effect_id: EffectId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptFailure {
    pub code: FailureCode,
    pub details: Option<Payload>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetryTimer {
    pub timer_id: TimerId,
    pub effect_id: EffectId,
    pub failed_attempt_id: AttemptId,
    pub next_attempt_number: AttemptNumber,
    pub delay_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExecutionState {
    AwaitingAttempt {
        activation: NodeActivation,
        attempt: NodeAttempt,
    },
    WaitingForRetry {
        activation: NodeActivation,
        attempt: NodeAttempt,
        failure: AttemptFailure,
        timer: RetryTimer,
    },
    Completed {
        output: Payload,
    },
    Failed {
        activation: NodeActivation,
        attempt: NodeAttempt,
        failure: AttemptFailure,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionSnapshot {
    pub execution_id: ExecutionId,
    pub plan_reference: PlanReference,
    pub revision: ExecutionRevision,
    pub state: ExecutionState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExecutionEvent {
    ExecutionStarted {
        event_id: EventId,
        execution_id: ExecutionId,
        plan_reference: PlanReference,
        input: Payload,
    },
    NodeAttemptSucceeded {
        event_id: EventId,
        execution_id: ExecutionId,
        expected_revision: ExecutionRevision,
        activation_id: NodeActivationId,
        attempt_id: AttemptId,
        attempt_number: AttemptNumber,
        effect_id: EffectId,
        node_id: NodeId,
        output: Payload,
    },
    NodeAttemptFailed {
        event_id: EventId,
        execution_id: ExecutionId,
        expected_revision: ExecutionRevision,
        activation_id: NodeActivationId,
        attempt_id: AttemptId,
        attempt_number: AttemptNumber,
        effect_id: EffectId,
        node_id: NodeId,
        failure: AttemptFailure,
    },
    TimerFired {
        event_id: EventId,
        execution_id: ExecutionId,
        expected_revision: ExecutionRevision,
        timer_id: TimerId,
        activation_id: NodeActivationId,
        next_attempt_number: AttemptNumber,
    },
}

impl ExecutionEvent {
    pub fn event_id(&self) -> &EventId {
        match self {
            Self::ExecutionStarted { event_id, .. }
            | Self::NodeAttemptSucceeded { event_id, .. }
            | Self::NodeAttemptFailed { event_id, .. }
            | Self::TimerFired { event_id, .. } => event_id,
        }
    }
    pub fn execution_id(&self) -> &ExecutionId {
        match self {
            Self::ExecutionStarted { execution_id, .. }
            | Self::NodeAttemptSucceeded { execution_id, .. }
            | Self::NodeAttemptFailed { execution_id, .. }
            | Self::TimerFired { execution_id, .. } => execution_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExecutionEffect {
    ExecuteNodeAttempt {
        effect_id: EffectId,
        activation_id: NodeActivationId,
        attempt_id: AttemptId,
        attempt_number: AttemptNumber,
        node_id: NodeId,
        input: Payload,
    },
    ScheduleTimer {
        effect_id: EffectId,
        timer_id: TimerId,
        activation_id: NodeActivationId,
        failed_attempt_id: AttemptId,
        next_attempt_number: AttemptNumber,
        delay_ms: u64,
    },
}
impl ExecutionEffect {
    pub fn effect_id(&self) -> &EffectId {
        match self {
            Self::ExecuteNodeAttempt { effect_id, .. } | Self::ScheduleTimer { effect_id, .. } => {
                effect_id
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub snapshot: ExecutionSnapshot,
    pub effects: Vec<ExecutionEffect>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TransitionError {
    #[error("executable plan failed its integrity check")]
    InvalidPlan,
    #[error("execution has not started")]
    ExecutionNotStarted,
    #[error("execution has already started")]
    ExecutionAlreadyStarted,
    #[error("terminal execution cannot accept events")]
    ExecutionAlreadyTerminal,
    #[error("execution plan reference does not match executable plan")]
    PlanReferenceMismatch,
    #[error("event execution id does not match snapshot")]
    ExecutionIdMismatch,
    #[error("event expects revision {expected:?}, but snapshot is {actual:?}")]
    StaleRevision {
        expected: ExecutionRevision,
        actual: ExecutionRevision,
    },
    #[error("attempt result does not match pending activation")]
    ActivationMismatch,
    #[error("attempt result does not match pending attempt")]
    AttemptMismatch,
    #[error("attempt result number does not match pending attempt")]
    AttemptNumberMismatch,
    #[error("attempt result does not match pending node")]
    NodeMismatch,
    #[error("attempt result does not match pending effect")]
    EffectMismatch,
    #[error("timer event does not match pending retry timer")]
    TimerMismatch,
    #[error("execution is not waiting for a retry timer")]
    NotWaitingForRetry,
    #[error("execution is not awaiting an attempt result")]
    NotAwaitingAttempt,
    #[error("snapshot is not a legal state for this plan")]
    InvalidSnapshot,
}

impl TransitionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPlan => "invalid-plan",
            Self::ExecutionNotStarted => "execution-not-started",
            Self::ExecutionAlreadyStarted => "execution-already-started",
            Self::ExecutionAlreadyTerminal => "execution-already-terminal",
            Self::PlanReferenceMismatch => "plan-reference-mismatch",
            Self::ExecutionIdMismatch => "execution-id-mismatch",
            Self::StaleRevision { .. } => "stale-revision",
            Self::ActivationMismatch => "activation-mismatch",
            Self::AttemptMismatch => "attempt-mismatch",
            Self::AttemptNumberMismatch => "attempt-number-mismatch",
            Self::NodeMismatch => "node-mismatch",
            Self::EffectMismatch => "effect-mismatch",
            Self::TimerMismatch => "timer-mismatch",
            Self::NotWaitingForRetry => "not-waiting-for-retry",
            Self::NotAwaitingAttempt => "not-awaiting-attempt",
            Self::InvalidSnapshot => "invalid-snapshot",
        }
    }
}

pub fn transition(
    plan: &ExecutableWorkflowPlan,
    snapshot: Option<&ExecutionSnapshot>,
    event: ExecutionEvent,
) -> Result<Transition, TransitionError> {
    if !plan.validate_integrity() {
        return Err(TransitionError::InvalidPlan);
    }
    match (snapshot, event) {
        (
            None,
            ExecutionEvent::ExecutionStarted {
                execution_id,
                plan_reference,
                input,
                ..
            },
        ) => {
            if plan_reference != plan.reference() {
                return Err(TransitionError::PlanReferenceMismatch);
            }
            advance_from_start(plan, execution_id, input)
        }
        (None, ExecutionEvent::NodeAttemptSucceeded { .. })
        | (None, ExecutionEvent::NodeAttemptFailed { .. })
        | (None, ExecutionEvent::TimerFired { .. }) => Err(TransitionError::ExecutionNotStarted),
        (Some(_), ExecutionEvent::ExecutionStarted { .. }) => {
            Err(TransitionError::ExecutionAlreadyStarted)
        }
        (Some(snapshot), event) => {
            validate_snapshot(plan, snapshot)?;
            match event {
                event @ (ExecutionEvent::NodeAttemptSucceeded { .. }
                | ExecutionEvent::NodeAttemptFailed { .. }) => {
                    apply_attempt_result(plan, snapshot, event)
                }
                event @ ExecutionEvent::TimerFired { .. } => {
                    fire_retry_timer(plan, snapshot, event)
                }
                ExecutionEvent::ExecutionStarted { .. } => unreachable!(),
            }
        }
    }
}

pub fn validate_snapshot(
    plan: &ExecutableWorkflowPlan,
    snapshot: &ExecutionSnapshot,
) -> Result<(), TransitionError> {
    if snapshot.plan_reference != plan.reference() {
        return Err(TransitionError::PlanReferenceMismatch);
    }
    if snapshot.revision.0 == 0 {
        return Err(TransitionError::InvalidSnapshot);
    }
    let activity_count = plan
        .nodes()
        .iter()
        .filter(|node| node.kind == NodeKind::Activity)
        .count() as u64;
    match &snapshot.state {
        ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } => {
            validate_activation_and_attempt(plan, &snapshot.execution_id, activation, attempt)?;
            if Some(snapshot.revision.0)
                != awaiting_revision(
                    activation.activation_revision.0,
                    attempt.attempt_number.value(),
                )
            {
                return Err(TransitionError::InvalidSnapshot);
            }
        }
        ExecutionState::WaitingForRetry {
            activation,
            attempt,
            failure,
            timer,
        } => {
            validate_activation_and_attempt(plan, &snapshot.execution_id, activation, attempt)?;
            let policy = plan
                .retry_policy_for(&activation.node_id)
                .ok_or(TransitionError::InvalidSnapshot)?;
            let next_attempt_number = attempt
                .attempt_number
                .checked_next()
                .ok_or(TransitionError::InvalidSnapshot)?;
            let delay_ms = policy
                .delay_after_failure(attempt.attempt_number.value())
                .ok_or(TransitionError::InvalidSnapshot)?;
            if Some(snapshot.revision.0)
                != after_failure_revision(
                    activation.activation_revision.0,
                    attempt.attempt_number.value(),
                )
                || attempt.attempt_number.value() >= policy.max_attempts
                || !policy.retryable_failure_codes.contains(&failure.code)
                || timer.failed_attempt_id != attempt.attempt_id
                || timer.next_attempt_number != next_attempt_number
                || timer.delay_ms != delay_ms
                || timer.timer_id
                    != TimerId::derive_retry(
                        &activation.activation_id,
                        &attempt.attempt_id,
                        next_attempt_number.value(),
                    )
                || timer.effect_id
                    != EffectId::derive_schedule_timer(
                        &timer.timer_id,
                        &activation.activation_id,
                        &attempt.attempt_id,
                        next_attempt_number.value(),
                    )
            {
                return Err(TransitionError::InvalidSnapshot);
            }
        }
        ExecutionState::Completed { .. } => {
            let base_revision = activity_count + 1;
            if snapshot.revision.0 < base_revision
                || !(snapshot.revision.0 - base_revision).is_multiple_of(2)
            {
                return Err(TransitionError::InvalidSnapshot);
            }
        }
        ExecutionState::Failed {
            activation,
            attempt,
            failure,
        } => {
            validate_activation_and_attempt(plan, &snapshot.execution_id, activation, attempt)?;
            let policy = plan
                .retry_policy_for(&activation.node_id)
                .ok_or(TransitionError::InvalidSnapshot)?;
            if Some(snapshot.revision.0)
                != after_failure_revision(
                    activation.activation_revision.0,
                    attempt.attempt_number.value(),
                )
                || (attempt.attempt_number.value() < policy.max_attempts
                    && policy.retryable_failure_codes.contains(&failure.code))
            {
                return Err(TransitionError::InvalidSnapshot);
            }
        }
    }
    Ok(())
}

fn advance_from_start(
    plan: &ExecutableWorkflowPlan,
    execution_id: ExecutionId,
    input: Payload,
) -> Result<Transition, TransitionError> {
    let revision = ExecutionRevision(1);
    let successor = plan
        .successor_of(plan.start_node())
        .ok_or(TransitionError::InvalidPlan)?;
    let node = plan.node(successor).ok_or(TransitionError::InvalidPlan)?;
    match node.kind {
        NodeKind::Activity => {
            awaiting_transition(plan, execution_id, revision, node.id.clone(), input)
        }
        NodeKind::Finish => Ok(completed_transition(plan, execution_id, revision, input)),
        NodeKind::Start => Err(TransitionError::InvalidPlan),
    }
}

fn apply_attempt_result(
    plan: &ExecutableWorkflowPlan,
    snapshot: &ExecutionSnapshot,
    event: ExecutionEvent,
) -> Result<Transition, TransitionError> {
    let result = AttemptResult::from_event(event)?;
    let execution_id = result.execution_id;
    if execution_id != snapshot.execution_id {
        return Err(TransitionError::ExecutionIdMismatch);
    }
    if result.expected_revision != snapshot.revision {
        return Err(TransitionError::StaleRevision {
            expected: result.expected_revision,
            actual: snapshot.revision,
        });
    }
    let (activation, attempt) = match &snapshot.state {
        ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } => (activation, attempt),
        ExecutionState::WaitingForRetry { .. } => {
            return Err(TransitionError::NotAwaitingAttempt);
        }
        ExecutionState::Completed { .. } | ExecutionState::Failed { .. } => {
            return Err(TransitionError::ExecutionAlreadyTerminal);
        }
    };
    if result.activation_id != activation.activation_id {
        return Err(TransitionError::ActivationMismatch);
    }
    if result.attempt_id != attempt.attempt_id {
        return Err(TransitionError::AttemptMismatch);
    }
    if result.attempt_number != attempt.attempt_number {
        return Err(TransitionError::AttemptNumberMismatch);
    }
    if result.effect_id != attempt.effect_id {
        return Err(TransitionError::EffectMismatch);
    }
    if result.node_id != activation.node_id {
        return Err(TransitionError::NodeMismatch);
    }
    let revision = ExecutionRevision(
        snapshot
            .revision
            .0
            .checked_add(1)
            .ok_or(TransitionError::InvalidSnapshot)?,
    );
    let output = match result.value {
        AttemptResultValue::Succeeded(output) => output,
        AttemptResultValue::Failed(failure) => {
            return after_attempt_failure(
                plan,
                execution_id,
                revision,
                activation.clone(),
                attempt.clone(),
                failure,
            );
        }
    };
    let (index, _) = plan
        .node_by_id(&activation.node_id)
        .ok_or(TransitionError::InvalidSnapshot)?;
    let successor = plan
        .successor_of(index)
        .ok_or(TransitionError::InvalidPlan)?;
    let successor_node = plan.node(successor).ok_or(TransitionError::InvalidPlan)?;
    match successor_node.kind {
        NodeKind::Activity => awaiting_transition(
            plan,
            execution_id,
            revision,
            successor_node.id.clone(),
            output,
        ),
        NodeKind::Finish => Ok(completed_transition(plan, execution_id, revision, output)),
        NodeKind::Start => Err(TransitionError::InvalidPlan),
    }
}

fn awaiting_transition(
    plan: &ExecutableWorkflowPlan,
    execution_id: ExecutionId,
    revision: ExecutionRevision,
    node_id: NodeId,
    input: Payload,
) -> Result<Transition, TransitionError> {
    let activation_id = NodeActivationId::derive(&execution_id, revision.0, &node_id);
    let activation = NodeActivation {
        activation_id,
        activation_revision: revision,
        node_id,
        input,
    };
    execute_attempt_transition(
        plan,
        execution_id,
        revision,
        activation,
        AttemptNumber::FIRST,
    )
}

fn execute_attempt_transition(
    plan: &ExecutableWorkflowPlan,
    execution_id: ExecutionId,
    revision: ExecutionRevision,
    activation: NodeActivation,
    attempt_number: AttemptNumber,
) -> Result<Transition, TransitionError> {
    let activation_id = activation.activation_id.clone();
    let node_id = activation.node_id.clone();
    let attempt_id = AttemptId::derive(&activation_id, attempt_number.value());
    let effect_id = EffectId::derive_execute_node_attempt(
        &activation_id,
        &attempt_id,
        attempt_number.value(),
        &node_id,
    );
    let attempt = NodeAttempt {
        attempt_id: attempt_id.clone(),
        attempt_number,
        effect_id: effect_id.clone(),
    };
    let effect = ExecutionEffect::ExecuteNodeAttempt {
        effect_id: effect_id.clone(),
        activation_id: activation_id.clone(),
        attempt_id,
        attempt_number,
        node_id: node_id.clone(),
        input: activation.input.clone(),
    };
    Ok(Transition {
        snapshot: ExecutionSnapshot {
            execution_id,
            plan_reference: plan.reference(),
            revision,
            state: ExecutionState::AwaitingAttempt {
                activation,
                attempt,
            },
        },
        effects: vec![effect],
    })
}

fn after_attempt_failure(
    plan: &ExecutableWorkflowPlan,
    execution_id: ExecutionId,
    revision: ExecutionRevision,
    activation: NodeActivation,
    attempt: NodeAttempt,
    failure: AttemptFailure,
) -> Result<Transition, TransitionError> {
    let policy = plan
        .retry_policy_for(&activation.node_id)
        .ok_or(TransitionError::InvalidPlan)?;
    let retryable = policy.retryable_failure_codes.contains(&failure.code)
        && attempt.attempt_number.value() < policy.max_attempts;
    if !retryable {
        return Ok(Transition {
            snapshot: ExecutionSnapshot {
                execution_id,
                plan_reference: plan.reference(),
                revision,
                state: ExecutionState::Failed {
                    activation,
                    attempt,
                    failure,
                },
            },
            effects: Vec::new(),
        });
    }

    let next_attempt_number = attempt
        .attempt_number
        .checked_next()
        .ok_or(TransitionError::InvalidPlan)?;
    let delay_ms = policy
        .delay_after_failure(attempt.attempt_number.value())
        .ok_or(TransitionError::InvalidPlan)?;
    let timer_id = TimerId::derive_retry(
        &activation.activation_id,
        &attempt.attempt_id,
        next_attempt_number.value(),
    );
    let effect_id = EffectId::derive_schedule_timer(
        &timer_id,
        &activation.activation_id,
        &attempt.attempt_id,
        next_attempt_number.value(),
    );
    let timer = RetryTimer {
        timer_id: timer_id.clone(),
        effect_id: effect_id.clone(),
        failed_attempt_id: attempt.attempt_id.clone(),
        next_attempt_number,
        delay_ms,
    };
    let effect = ExecutionEffect::ScheduleTimer {
        effect_id,
        timer_id,
        activation_id: activation.activation_id.clone(),
        failed_attempt_id: attempt.attempt_id.clone(),
        next_attempt_number,
        delay_ms,
    };
    Ok(Transition {
        snapshot: ExecutionSnapshot {
            execution_id,
            plan_reference: plan.reference(),
            revision,
            state: ExecutionState::WaitingForRetry {
                activation,
                attempt,
                failure,
                timer,
            },
        },
        effects: vec![effect],
    })
}

fn fire_retry_timer(
    plan: &ExecutableWorkflowPlan,
    snapshot: &ExecutionSnapshot,
    event: ExecutionEvent,
) -> Result<Transition, TransitionError> {
    let ExecutionEvent::TimerFired {
        execution_id,
        expected_revision,
        timer_id,
        activation_id,
        next_attempt_number,
        ..
    } = event
    else {
        unreachable!()
    };
    if execution_id != snapshot.execution_id {
        return Err(TransitionError::ExecutionIdMismatch);
    }
    if expected_revision != snapshot.revision {
        return Err(TransitionError::StaleRevision {
            expected: expected_revision,
            actual: snapshot.revision,
        });
    }
    let (activation, timer) = match &snapshot.state {
        ExecutionState::WaitingForRetry {
            activation, timer, ..
        } => (activation, timer),
        ExecutionState::Completed { .. } | ExecutionState::Failed { .. } => {
            return Err(TransitionError::ExecutionAlreadyTerminal);
        }
        ExecutionState::AwaitingAttempt { .. } => {
            return Err(TransitionError::NotWaitingForRetry);
        }
    };
    if timer_id != timer.timer_id {
        return Err(TransitionError::TimerMismatch);
    }
    if activation_id != activation.activation_id {
        return Err(TransitionError::ActivationMismatch);
    }
    if next_attempt_number != timer.next_attempt_number {
        return Err(TransitionError::AttemptNumberMismatch);
    }
    let revision = ExecutionRevision(
        snapshot
            .revision
            .0
            .checked_add(1)
            .ok_or(TransitionError::InvalidSnapshot)?,
    );
    execute_attempt_transition(
        plan,
        execution_id,
        revision,
        activation.clone(),
        next_attempt_number,
    )
}

fn validate_activation_and_attempt(
    plan: &ExecutableWorkflowPlan,
    execution_id: &ExecutionId,
    activation: &NodeActivation,
    attempt: &NodeAttempt,
) -> Result<(), TransitionError> {
    let (node_index, expected_node) = plan
        .node_by_id(&activation.node_id)
        .ok_or(TransitionError::InvalidSnapshot)?;
    let policy = expected_node
        .retry_policy
        .as_ref()
        .ok_or(TransitionError::InvalidSnapshot)?;
    let base_activation_revision = u64::from(node_index.value());
    if expected_node.kind != NodeKind::Activity
        || activation.activation_revision.0 < base_activation_revision
        || !(activation.activation_revision.0 - base_activation_revision).is_multiple_of(2)
        || attempt.attempt_number.value() > policy.max_attempts
        || activation.activation_id
            != NodeActivationId::derive(
                execution_id,
                activation.activation_revision.0,
                &activation.node_id,
            )
        || attempt.attempt_id
            != AttemptId::derive(&activation.activation_id, attempt.attempt_number.value())
        || attempt.effect_id
            != EffectId::derive_execute_node_attempt(
                &activation.activation_id,
                &attempt.attempt_id,
                attempt.attempt_number.value(),
                &activation.node_id,
            )
    {
        return Err(TransitionError::InvalidSnapshot);
    }
    Ok(())
}

fn awaiting_revision(activation_revision: u64, attempt_number: u32) -> Option<u64> {
    let retry_events = u64::from(attempt_number.checked_sub(1)?).checked_mul(2)?;
    activation_revision.checked_add(retry_events)
}

fn after_failure_revision(activation_revision: u64, attempt_number: u32) -> Option<u64> {
    let accepted_events = u64::from(attempt_number).checked_mul(2)?.checked_sub(1)?;
    activation_revision.checked_add(accepted_events)
}

struct AttemptResult {
    execution_id: ExecutionId,
    expected_revision: ExecutionRevision,
    activation_id: NodeActivationId,
    attempt_id: AttemptId,
    attempt_number: AttemptNumber,
    effect_id: EffectId,
    node_id: NodeId,
    value: AttemptResultValue,
}

enum AttemptResultValue {
    Succeeded(Payload),
    Failed(AttemptFailure),
}

impl AttemptResult {
    fn from_event(event: ExecutionEvent) -> Result<Self, TransitionError> {
        match event {
            ExecutionEvent::ExecutionStarted { .. } => {
                Err(TransitionError::ExecutionAlreadyStarted)
            }
            ExecutionEvent::NodeAttemptSucceeded {
                execution_id,
                expected_revision,
                activation_id,
                attempt_id,
                attempt_number,
                effect_id,
                node_id,
                output,
                ..
            } => Ok(Self {
                execution_id,
                expected_revision,
                activation_id,
                attempt_id,
                attempt_number,
                effect_id,
                node_id,
                value: AttemptResultValue::Succeeded(output),
            }),
            ExecutionEvent::NodeAttemptFailed {
                execution_id,
                expected_revision,
                activation_id,
                attempt_id,
                attempt_number,
                effect_id,
                node_id,
                failure,
                ..
            } => Ok(Self {
                execution_id,
                expected_revision,
                activation_id,
                attempt_id,
                attempt_number,
                effect_id,
                node_id,
                value: AttemptResultValue::Failed(failure),
            }),
            ExecutionEvent::TimerFired { .. } => Err(TransitionError::NotAwaitingAttempt),
        }
    }
}

fn completed_transition(
    plan: &ExecutableWorkflowPlan,
    execution_id: ExecutionId,
    revision: ExecutionRevision,
    output: Payload,
) -> Transition {
    Transition {
        snapshot: ExecutionSnapshot {
            execution_id,
            plan_reference: plan.reference(),
            revision,
            state: ExecutionState::Completed { output },
        },
        effects: Vec::new(),
    }
}

#[cfg(test)]
mod attempt_tests {
    use std::collections::BTreeSet;

    use super::*;
    use flower_compiler::{EdgeDefinition, NodeDefinition, WorkflowDefinition, compile};
    use flower_plan::{BackoffPolicy, EdgeId, RetryPolicy};

    fn id<T: std::str::FromStr>(value: &str) -> T
    where
        T::Err: std::fmt::Debug,
    {
        value.parse().unwrap()
    }

    fn payload(value: &str) -> Payload {
        Payload {
            media_type: "text/plain".into(),
            bytes: value.as_bytes().to_vec(),
        }
    }

    fn plan() -> ExecutableWorkflowPlan {
        compile(WorkflowDefinition {
            id: id("flow"),
            nodes: vec![
                NodeDefinition {
                    id: id("start"),
                    kind: NodeKind::Start,
                    retry_policy: None,
                },
                NodeDefinition {
                    id: id("work"),
                    kind: NodeKind::Activity,
                    retry_policy: None,
                },
                NodeDefinition {
                    id: id("finish"),
                    kind: NodeKind::Finish,
                    retry_policy: None,
                },
            ],
            edges: vec![
                EdgeDefinition {
                    id: EdgeId::new("a").unwrap(),
                    source: id("start"),
                    target: id("work"),
                },
                EdgeDefinition {
                    id: EdgeId::new("b").unwrap(),
                    source: id("work"),
                    target: id("finish"),
                },
            ],
        })
        .unwrap()
    }

    fn retry_plan() -> ExecutableWorkflowPlan {
        compile(WorkflowDefinition {
            id: id("retry-flow"),
            nodes: vec![
                NodeDefinition {
                    id: id("start"),
                    kind: NodeKind::Start,
                    retry_policy: None,
                },
                NodeDefinition {
                    id: id("work"),
                    kind: NodeKind::Activity,
                    retry_policy: Some(RetryPolicy {
                        max_attempts: 3,
                        retryable_failure_codes: BTreeSet::from([id("worker.failed")]),
                        backoff: BackoffPolicy::Exponential {
                            initial_delay_ms: 10,
                            multiplier: 3,
                            maximum_delay_ms: 50,
                        },
                    }),
                },
                NodeDefinition {
                    id: id("finish"),
                    kind: NodeKind::Finish,
                    retry_policy: None,
                },
            ],
            edges: vec![
                EdgeDefinition {
                    id: id("a"),
                    source: id("start"),
                    target: id("work"),
                },
                EdgeDefinition {
                    id: id("b"),
                    source: id("work"),
                    target: id("finish"),
                },
            ],
        })
        .unwrap()
    }

    fn started(plan: &ExecutableWorkflowPlan) -> ExecutionEvent {
        ExecutionEvent::ExecutionStarted {
            event_id: id("event-1"),
            execution_id: id("execution"),
            plan_reference: plan.reference(),
            input: payload("input"),
        }
    }

    fn attempt_event(snapshot: &ExecutionSnapshot, succeeded: bool) -> ExecutionEvent {
        let ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } = &snapshot.state
        else {
            panic!("expected pending attempt")
        };
        if succeeded {
            ExecutionEvent::NodeAttemptSucceeded {
                event_id: id("event-2"),
                execution_id: snapshot.execution_id.clone(),
                expected_revision: snapshot.revision,
                activation_id: activation.activation_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
                attempt_number: attempt.attempt_number,
                effect_id: attempt.effect_id.clone(),
                node_id: activation.node_id.clone(),
                output: payload("output"),
            }
        } else {
            ExecutionEvent::NodeAttemptFailed {
                event_id: id("event-2"),
                execution_id: snapshot.execution_id.clone(),
                expected_revision: snapshot.revision,
                activation_id: activation.activation_id.clone(),
                attempt_id: attempt.attempt_id.clone(),
                attempt_number: attempt.attempt_number,
                effect_id: attempt.effect_id.clone(),
                node_id: activation.node_id.clone(),
                failure: AttemptFailure {
                    code: id("worker.failed"),
                    details: Some(payload("details")),
                },
            }
        }
    }

    fn timer_fired(snapshot: &ExecutionSnapshot) -> ExecutionEvent {
        let ExecutionState::WaitingForRetry {
            activation, timer, ..
        } = &snapshot.state
        else {
            panic!("expected retry timer")
        };
        ExecutionEvent::TimerFired {
            event_id: id("timer-fired"),
            execution_id: snapshot.execution_id.clone(),
            expected_revision: snapshot.revision,
            timer_id: timer.timer_id.clone(),
            activation_id: activation.activation_id.clone(),
            next_attempt_number: timer.next_attempt_number,
        }
    }

    #[test]
    fn first_activation_creates_exactly_one_first_attempt() {
        let plan = plan();
        let transition = transition(&plan, None, started(&plan)).unwrap();
        let ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } = &transition.snapshot.state
        else {
            panic!("expected pending attempt")
        };
        assert_eq!(attempt.attempt_number, AttemptNumber::FIRST);
        assert_eq!(transition.effects.len(), 1);
        assert_eq!(
            activation.activation_id,
            NodeActivationId::derive(&transition.snapshot.execution_id, 1, &activation.node_id),
        );
    }

    #[test]
    fn success_completes_and_failure_is_terminal_without_effects() {
        let plan = plan();
        let first = transition(&plan, None, started(&plan)).unwrap();
        let completed = transition(
            &plan,
            Some(&first.snapshot),
            attempt_event(&first.snapshot, true),
        )
        .unwrap();
        assert!(matches!(
            completed.snapshot.state,
            ExecutionState::Completed { .. }
        ));
        assert!(completed.effects.is_empty());

        let failed = transition(
            &plan,
            Some(&first.snapshot),
            attempt_event(&first.snapshot, false),
        )
        .unwrap();
        assert!(matches!(
            failed.snapshot.state,
            ExecutionState::Failed { .. }
        ));
        assert!(failed.effects.is_empty());
    }

    #[test]
    fn mismatch_errors_follow_the_normative_order() {
        let plan = plan();
        let first = transition(&plan, None, started(&plan)).unwrap();
        let mut event = attempt_event(&first.snapshot, true);
        let ExecutionEvent::NodeAttemptSucceeded {
            execution_id,
            expected_revision,
            activation_id,
            ..
        } = &mut event
        else {
            unreachable!()
        };
        *execution_id = id("wrong-execution");
        *expected_revision = ExecutionRevision(0);
        *activation_id = id("wrong-activation");
        assert_eq!(
            transition(&plan, Some(&first.snapshot), event)
                .unwrap_err()
                .code(),
            "execution-id-mismatch"
        );
    }

    #[test]
    fn forged_snapshot_is_rejected() {
        let plan = plan();
        let mut first = transition(&plan, None, started(&plan)).unwrap().snapshot;
        let ExecutionState::AwaitingAttempt { attempt, .. } = &mut first.state else {
            unreachable!()
        };
        attempt.attempt_number = AttemptNumber::new(2).unwrap();
        assert_eq!(
            validate_snapshot(&plan, &first),
            Err(TransitionError::InvalidSnapshot)
        );
    }

    #[test]
    fn retryable_failures_schedule_deterministic_timers_until_exhausted() {
        let plan = retry_plan();
        let first = transition(&plan, None, started(&plan)).unwrap();
        let waiting_one = transition(
            &plan,
            Some(&first.snapshot),
            attempt_event(&first.snapshot, false),
        )
        .unwrap();
        let ExecutionState::WaitingForRetry {
            activation, timer, ..
        } = &waiting_one.snapshot.state
        else {
            panic!("expected retry timer")
        };
        assert_eq!(timer.delay_ms, 10);
        assert_eq!(timer.next_attempt_number, AttemptNumber::new(2).unwrap());
        assert!(matches!(
            waiting_one.effects.as_slice(),
            [ExecutionEffect::ScheduleTimer { delay_ms: 10, .. }]
        ));
        let activation_id = activation.activation_id.clone();

        let second = transition(
            &plan,
            Some(&waiting_one.snapshot),
            timer_fired(&waiting_one.snapshot),
        )
        .unwrap();
        let ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } = &second.snapshot.state
        else {
            panic!("expected second attempt")
        };
        assert_eq!(activation.activation_id, activation_id);
        assert_eq!(attempt.attempt_number, AttemptNumber::new(2).unwrap());

        let waiting_two = transition(
            &plan,
            Some(&second.snapshot),
            attempt_event(&second.snapshot, false),
        )
        .unwrap();
        let ExecutionState::WaitingForRetry { timer, .. } = &waiting_two.snapshot.state else {
            panic!("expected second retry timer")
        };
        assert_eq!(timer.delay_ms, 30);

        let third = transition(
            &plan,
            Some(&waiting_two.snapshot),
            timer_fired(&waiting_two.snapshot),
        )
        .unwrap();
        let exhausted = transition(
            &plan,
            Some(&third.snapshot),
            attempt_event(&third.snapshot, false),
        )
        .unwrap();
        assert!(matches!(
            exhausted.snapshot.state,
            ExecutionState::Failed { .. }
        ));
        assert!(exhausted.effects.is_empty());
    }

    #[test]
    fn non_retryable_failure_does_not_schedule_a_timer() {
        let plan = retry_plan();
        let first = transition(&plan, None, started(&plan)).unwrap();
        let mut failure_event = attempt_event(&first.snapshot, false);
        let ExecutionEvent::NodeAttemptFailed { failure, .. } = &mut failure_event else {
            unreachable!()
        };
        failure.code = id("worker.rejected");
        let failed = transition(&plan, Some(&first.snapshot), failure_event).unwrap();
        assert!(matches!(
            failed.snapshot.state,
            ExecutionState::Failed { .. }
        ));
        assert!(failed.effects.is_empty());
    }

    #[test]
    fn timer_identity_is_validated_before_activation_and_attempt_number() {
        let plan = retry_plan();
        let first = transition(&plan, None, started(&plan)).unwrap();
        let waiting = transition(
            &plan,
            Some(&first.snapshot),
            attempt_event(&first.snapshot, false),
        )
        .unwrap();
        let mut event = timer_fired(&waiting.snapshot);
        let ExecutionEvent::TimerFired {
            timer_id,
            activation_id,
            next_attempt_number,
            ..
        } = &mut event
        else {
            unreachable!()
        };
        *timer_id = id("wrong-timer");
        *activation_id = id("wrong-activation");
        *next_attempt_number = AttemptNumber::FIRST;
        assert_eq!(
            transition(&plan, Some(&waiting.snapshot), event)
                .unwrap_err()
                .code(),
            "timer-mismatch"
        );
    }
}
