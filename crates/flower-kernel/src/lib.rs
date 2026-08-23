use flower_plan::{
    EffectId, EventId, ExecutableWorkflowPlan, ExecutionId, NodeId, NodeKind, PlanReference,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExecutionState {
    AwaitingNode {
        node_id: NodeId,
        effect_id: EffectId,
        input: Payload,
    },
    Completed {
        output: Payload,
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
    NodeCompleted {
        event_id: EventId,
        execution_id: ExecutionId,
        expected_revision: ExecutionRevision,
        effect_id: EffectId,
        node_id: NodeId,
        output: Payload,
    },
}

impl ExecutionEvent {
    pub fn event_id(&self) -> &EventId {
        match self {
            Self::ExecutionStarted { event_id, .. } | Self::NodeCompleted { event_id, .. } => {
                event_id
            }
        }
    }
    pub fn execution_id(&self) -> &ExecutionId {
        match self {
            Self::ExecutionStarted { execution_id, .. }
            | Self::NodeCompleted { execution_id, .. } => execution_id,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExecutionEffect {
    ExecuteNode {
        effect_id: EffectId,
        execution_id: ExecutionId,
        node_id: NodeId,
        input: Payload,
    },
}
impl ExecutionEffect {
    pub fn effect_id(&self) -> &EffectId {
        match self {
            Self::ExecuteNode { effect_id, .. } => effect_id,
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
    #[error("completed execution cannot accept events")]
    ExecutionAlreadyCompleted,
    #[error("execution plan reference does not match executable plan")]
    PlanReferenceMismatch,
    #[error("event execution id does not match snapshot")]
    ExecutionIdMismatch,
    #[error("event expects revision {expected:?}, but snapshot is {actual:?}")]
    StaleRevision {
        expected: ExecutionRevision,
        actual: ExecutionRevision,
    },
    #[error("node completion does not match pending node")]
    NodeMismatch,
    #[error("node completion does not match pending effect")]
    EffectMismatch,
    #[error("snapshot is not a legal state for this plan")]
    InvalidSnapshot,
}

impl TransitionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPlan => "invalid-plan",
            Self::ExecutionNotStarted => "execution-not-started",
            Self::ExecutionAlreadyStarted => "execution-already-started",
            Self::ExecutionAlreadyCompleted => "execution-already-completed",
            Self::PlanReferenceMismatch => "plan-reference-mismatch",
            Self::ExecutionIdMismatch => "execution-id-mismatch",
            Self::StaleRevision { .. } => "stale-revision",
            Self::NodeMismatch => "node-mismatch",
            Self::EffectMismatch => "effect-mismatch",
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
        (None, ExecutionEvent::NodeCompleted { .. }) => Err(TransitionError::ExecutionNotStarted),
        (Some(_), ExecutionEvent::ExecutionStarted { .. }) => {
            Err(TransitionError::ExecutionAlreadyStarted)
        }
        (Some(snapshot), event @ ExecutionEvent::NodeCompleted { .. }) => {
            validate_snapshot(plan, snapshot)?;
            complete_node(plan, snapshot, event)
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
        ExecutionState::AwaitingNode {
            node_id, effect_id, ..
        } => {
            let expected_node = plan
                .activity_at_revision(snapshot.revision.0)
                .ok_or(TransitionError::InvalidSnapshot)?;
            if &expected_node.id != node_id
                || *effect_id
                    != derive_effect_id(&snapshot.execution_id, snapshot.revision, node_id)
            {
                return Err(TransitionError::InvalidSnapshot);
            }
        }
        ExecutionState::Completed { .. } if snapshot.revision.0 != activity_count + 1 => {
            return Err(TransitionError::InvalidSnapshot);
        }
        ExecutionState::Completed { .. } => {}
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

fn complete_node(
    plan: &ExecutableWorkflowPlan,
    snapshot: &ExecutionSnapshot,
    event: ExecutionEvent,
) -> Result<Transition, TransitionError> {
    let ExecutionEvent::NodeCompleted {
        execution_id,
        expected_revision,
        effect_id,
        node_id,
        output,
        ..
    } = event
    else {
        return Err(TransitionError::ExecutionAlreadyStarted);
    };
    if execution_id != snapshot.execution_id {
        return Err(TransitionError::ExecutionIdMismatch);
    }
    let ExecutionState::AwaitingNode {
        node_id: pending_node,
        effect_id: pending_effect,
        ..
    } = &snapshot.state
    else {
        return Err(TransitionError::ExecutionAlreadyCompleted);
    };
    if expected_revision != snapshot.revision {
        return Err(TransitionError::StaleRevision {
            expected: expected_revision,
            actual: snapshot.revision,
        });
    }
    if &node_id != pending_node {
        return Err(TransitionError::NodeMismatch);
    }
    if &effect_id != pending_effect {
        return Err(TransitionError::EffectMismatch);
    }
    let (index, _) = plan
        .node_by_id(&node_id)
        .ok_or(TransitionError::InvalidSnapshot)?;
    let successor = plan
        .successor_of(index)
        .ok_or(TransitionError::InvalidPlan)?;
    let successor_node = plan.node(successor).ok_or(TransitionError::InvalidPlan)?;
    let revision = ExecutionRevision(
        snapshot
            .revision
            .0
            .checked_add(1)
            .ok_or(TransitionError::InvalidSnapshot)?,
    );
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
    let effect_id = derive_effect_id(&execution_id, revision, &node_id);
    let effect = ExecutionEffect::ExecuteNode {
        effect_id: effect_id.clone(),
        execution_id: execution_id.clone(),
        node_id: node_id.clone(),
        input: input.clone(),
    };
    Ok(Transition {
        snapshot: ExecutionSnapshot {
            execution_id,
            plan_reference: plan.reference(),
            revision,
            state: ExecutionState::AwaitingNode {
                node_id,
                effect_id,
                input,
            },
        },
        effects: vec![effect],
    })
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

pub fn derive_effect_id(
    execution_id: &ExecutionId,
    revision: ExecutionRevision,
    node_id: &NodeId,
) -> EffectId {
    const DOMAIN_SEPARATOR: &[u8] = b"flower.effect-id.v0.1\0";

    let mut canonical = Sha256::new();
    canonical.update(DOMAIN_SEPARATOR);
    hash_length_prefixed(&mut canonical, execution_id.as_str().as_bytes());
    canonical.update(revision.0.to_be_bytes());
    hash_length_prefixed(&mut canonical, node_id.as_str().as_bytes());
    let digest = canonical.finalize();
    EffectId::new(format!("effect-{digest:x}"))
        .expect("a SHA-256 effect id always satisfies the identifier grammar")
}

fn hash_length_prefixed(hasher: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("identifier length fits into u64");
    hasher.update(length.to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use flower_compiler::{EdgeDefinition, NodeDefinition, WorkflowDefinition, compile};
    use flower_plan::{EdgeId, WorkflowId};
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
            id: WorkflowId::new("flow").unwrap(),
            nodes: vec![
                NodeDefinition {
                    id: id("start"),
                    kind: NodeKind::Start,
                },
                NodeDefinition {
                    id: id("one"),
                    kind: NodeKind::Activity,
                },
                NodeDefinition {
                    id: id("two"),
                    kind: NodeKind::Activity,
                },
                NodeDefinition {
                    id: id("finish"),
                    kind: NodeKind::Finish,
                },
            ],
            edges: vec![
                EdgeDefinition {
                    id: EdgeId::new("a").unwrap(),
                    source: id("start"),
                    target: id("one"),
                },
                EdgeDefinition {
                    id: EdgeId::new("b").unwrap(),
                    source: id("one"),
                    target: id("two"),
                },
                EdgeDefinition {
                    id: EdgeId::new("c").unwrap(),
                    source: id("two"),
                    target: id("finish"),
                },
            ],
        })
        .unwrap()
    }

    #[test]
    fn linear_workflow_is_deterministic_and_rejects_duplicates() {
        let plan = plan();
        let started = ExecutionEvent::ExecutionStarted {
            event_id: id("event-1"),
            execution_id: id("execution-1"),
            plan_reference: plan.reference(),
            input: payload("in"),
        };
        let first = transition(&plan, None, started.clone()).unwrap();
        assert_eq!(first, transition(&plan, None, started).unwrap());
        let ExecutionState::AwaitingNode {
            node_id, effect_id, ..
        } = &first.snapshot.state
        else {
            panic!()
        };
        let completion = ExecutionEvent::NodeCompleted {
            event_id: id("event-2"),
            execution_id: id("execution-1"),
            expected_revision: first.snapshot.revision,
            effect_id: effect_id.clone(),
            node_id: node_id.clone(),
            output: payload("out"),
        };
        let second = transition(&plan, Some(&first.snapshot), completion.clone()).unwrap();
        assert_eq!(
            transition(&plan, Some(&second.snapshot), completion)
                .unwrap_err()
                .code(),
            "stale-revision"
        );
    }

    #[test]
    fn forged_snapshot_never_panics() {
        let plan = plan();
        let snapshot = ExecutionSnapshot {
            execution_id: id("x"),
            plan_reference: plan.reference(),
            revision: ExecutionRevision(u64::MAX),
            state: ExecutionState::Completed {
                output: payload("x"),
            },
        };
        assert_eq!(
            validate_snapshot(&plan, &snapshot),
            Err(TransitionError::InvalidSnapshot)
        );
    }

    #[test]
    fn every_linear_length_completes_in_order() {
        for activity_count in 0..32 {
            let mut nodes = vec![NodeDefinition {
                id: id("start"),
                kind: NodeKind::Start,
            }];
            nodes.extend((0..activity_count).map(|index| NodeDefinition {
                id: NodeId::new(format!("node-{index}")).unwrap(),
                kind: NodeKind::Activity,
            }));
            nodes.push(NodeDefinition {
                id: id("finish"),
                kind: NodeKind::Finish,
            });
            let edges = nodes
                .windows(2)
                .enumerate()
                .map(|(index, pair)| EdgeDefinition {
                    id: EdgeId::new(format!("edge-{index}")).unwrap(),
                    source: pair[0].id.clone(),
                    target: pair[1].id.clone(),
                })
                .collect();
            let plan = compile(WorkflowDefinition {
                id: WorkflowId::new(format!("linear-{activity_count}")).unwrap(),
                nodes,
                edges,
            })
            .unwrap();
            let mut current = transition(
                &plan,
                None,
                ExecutionEvent::ExecutionStarted {
                    event_id: id("start-event"),
                    execution_id: id("execution"),
                    plan_reference: plan.reference(),
                    input: payload("input"),
                },
            )
            .unwrap();
            let mut completed = 0;
            while let ExecutionState::AwaitingNode {
                node_id, effect_id, ..
            } = &current.snapshot.state
            {
                current = transition(
                    &plan,
                    Some(&current.snapshot),
                    ExecutionEvent::NodeCompleted {
                        event_id: EventId::new(format!("event-{completed}")).unwrap(),
                        execution_id: current.snapshot.execution_id.clone(),
                        expected_revision: current.snapshot.revision,
                        effect_id: effect_id.clone(),
                        node_id: node_id.clone(),
                        output: payload("output"),
                    },
                )
                .unwrap();
                completed += 1;
            }
            assert_eq!(completed, activity_count);
            assert!(matches!(
                current.snapshot.state,
                ExecutionState::Completed { .. }
            ));
        }
    }

    #[test]
    fn fuzzed_external_snapshots_never_panic() {
        let plan = plan();
        for seed in 0..4096_u64 {
            let snapshot = ExecutionSnapshot {
                execution_id: ExecutionId::new(format!("execution-{seed}")).unwrap(),
                plan_reference: if seed % 5 == 0 {
                    PlanReference {
                        specification_version: flower_plan::SpecificationVersion {
                            major: 99,
                            minor: 99,
                        },
                        workflow_id: WorkflowId::new(format!("workflow-{seed}")).unwrap(),
                        fingerprint: flower_plan::PlanFingerprint::from_sha256(format!(
                            "{seed:064x}"
                        ))
                        .unwrap(),
                    }
                } else {
                    plan.reference()
                },
                revision: ExecutionRevision(seed),
                state: if seed % 2 == 0 {
                    ExecutionState::AwaitingNode {
                        node_id: NodeId::new(format!("node-{seed}")).unwrap(),
                        effect_id: EffectId::new(format!("effect-{seed}")).unwrap(),
                        input: payload("input"),
                    }
                } else {
                    ExecutionState::Completed {
                        output: payload("output"),
                    }
                },
            };
            let event = ExecutionEvent::NodeCompleted {
                event_id: EventId::new(format!("event-{seed}")).unwrap(),
                execution_id: snapshot.execution_id.clone(),
                expected_revision: ExecutionRevision(seed.wrapping_sub(1)),
                effect_id: EffectId::new(format!("effect-{seed}")).unwrap(),
                node_id: NodeId::new(format!("node-{seed}")).unwrap(),
                output: payload("output"),
            };
            let _ = transition(&plan, Some(&snapshot), event);
        }
    }

    #[test]
    fn effect_ids_use_unambiguous_domain_separated_encoding() {
        let first = derive_effect_id(&id("tenant"), ExecutionRevision(1), &id("node.2.job"));
        let second = derive_effect_id(&id("tenant.1.node"), ExecutionRevision(2), &id("job"));

        assert_eq!(
            first.as_str(),
            "effect-06c8f27d9de5825234bbf9754c13a557b394839002c05df3a7f7bb74af956655"
        );
        assert_eq!(
            second.as_str(),
            "effect-156aba9fe41440419600d77ec482056ecfa03c671a3770c6d32d243fc4fe655e"
        );
        assert_ne!(first, second);
    }
}
