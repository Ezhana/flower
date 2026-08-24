use std::{env, fs, path::PathBuf, str::FromStr};

use flower_engine::{
    AttemptFailure, AttemptNumber, ExecutionEvent, ExecutionRevision, ExecutionSnapshot,
    ExecutionState, Payload, transition,
};
use flower_engine::{
    AttemptId, EdgeId, EffectId, ExecutionId, FailureCode, NodeActivationId, NodeId, NodeKind,
    WorkflowId,
};
use flower_engine::{EdgeDefinition, NodeDefinition, WorkflowDefinition, compile};
use flower_engine_tests::{
    CompileExpectation, EngineFixture, ExpectedEngineError, ExpectedPlan, TransitionExpectation,
    TransitionStep,
};

fn id<T: FromStr>(value: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    value.parse().unwrap()
}

fn payload(value: &str) -> Payload {
    Payload {
        media_type: "text/plain".to_owned(),
        bytes: value.as_bytes().to_vec(),
    }
}

fn main() {
    let output = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: generate_attempt_lifecycle <output-file>");
    let definition = WorkflowDefinition {
        id: WorkflowId::new("attempt-lifecycle").unwrap(),
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
                id: EdgeId::new("start-work").unwrap(),
                source: id("start"),
                target: id("work"),
            },
            EdgeDefinition {
                id: EdgeId::new("work-finish").unwrap(),
                source: id("work"),
                target: id("finish"),
            },
        ],
    };
    let plan = compile(definition.clone()).unwrap();
    let started = ExecutionEvent::ExecutionStarted {
        execution_id: id("execution"),
        plan_reference: plan.reference(),
        input: payload("input"),
    };
    let first = transition(&plan, None, started.clone()).unwrap();
    let ExecutionState::AwaitingAttempt {
        activation,
        attempt,
    } = &first.snapshot.state
    else {
        panic!("expected pending attempt")
    };
    let succeeded = ExecutionEvent::NodeAttemptSucceeded {
        execution_id: first.snapshot.execution_id.clone(),
        expected_revision: first.snapshot.revision,
        activation_id: activation.activation_id.clone(),
        attempt_id: attempt.attempt_id.clone(),
        attempt_number: attempt.attempt_number,
        effect_id: attempt.effect_id.clone(),
        node_id: activation.node_id.clone(),
        output: payload("output"),
    };
    let failed = ExecutionEvent::NodeAttemptFailed {
        execution_id: first.snapshot.execution_id.clone(),
        expected_revision: first.snapshot.revision,
        activation_id: activation.activation_id.clone(),
        attempt_id: attempt.attempt_id.clone(),
        attempt_number: attempt.attempt_number,
        effect_id: attempt.effect_id.clone(),
        node_id: activation.node_id.clone(),
        failure: AttemptFailure {
            code: FailureCode::new("worker.failed").unwrap(),
            details: Some(payload("details")),
        },
    };

    let mut steps = vec![step(&plan, None, started)];
    steps.push(step(&plan, Some(first.snapshot.clone()), succeeded.clone()));
    steps.push(step(&plan, Some(first.snapshot.clone()), failed.clone()));

    let mut wrong_execution = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded { execution_id, .. } = &mut wrong_execution {
        *execution_id = ExecutionId::new("wrong-execution").unwrap();
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), wrong_execution));

    let mut stale = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded {
        expected_revision, ..
    } = &mut stale
    {
        *expected_revision = ExecutionRevision(0);
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), stale));

    let mut wrong_activation = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded { activation_id, .. } = &mut wrong_activation {
        *activation_id = NodeActivationId::new("wrong-activation").unwrap();
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), wrong_activation));

    let mut wrong_attempt = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded { attempt_id, .. } = &mut wrong_attempt {
        *attempt_id = AttemptId::new("wrong-attempt").unwrap();
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), wrong_attempt));

    let mut wrong_number = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded { attempt_number, .. } = &mut wrong_number {
        *attempt_number = AttemptNumber::new(2).unwrap();
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), wrong_number));

    let mut wrong_effect = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded { effect_id, .. } = &mut wrong_effect {
        *effect_id = EffectId::new("wrong-effect").unwrap();
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), wrong_effect));

    let mut wrong_node = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded { node_id, .. } = &mut wrong_node {
        *node_id = NodeId::new("wrong-node").unwrap();
    }
    steps.push(step(&plan, Some(first.snapshot.clone()), wrong_node));

    let failed_transition = transition(&plan, Some(&first.snapshot), failed).unwrap();
    let mut terminal_event = succeeded.clone();
    if let ExecutionEvent::NodeAttemptSucceeded {
        expected_revision, ..
    } = &mut terminal_event
    {
        *expected_revision = failed_transition.snapshot.revision;
    }
    steps.push(step(
        &plan,
        Some(failed_transition.snapshot),
        terminal_event,
    ));

    let mut forged_snapshot = first.snapshot.clone();
    if let ExecutionState::AwaitingAttempt { attempt, .. } = &mut forged_snapshot.state {
        attempt.attempt_id = AttemptId::new("forged-attempt").unwrap();
    }
    steps.push(step(&plan, Some(forged_snapshot), succeeded));

    let fixture = EngineFixture {
        schema_version: "flower.engine-fixture/v1".to_owned(),
        name: "attempt-lifecycle".to_owned(),
        definition,
        expected_compile: CompileExpectation::Plan {
            plan: ExpectedPlan::from_plan(&plan),
        },
        steps,
    };
    fs::create_dir_all(output.parent().expect("output has a parent")).unwrap();
    fs::write(output, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
}

fn step(
    plan: &flower_engine::ExecutableWorkflowPlan,
    snapshot: Option<ExecutionSnapshot>,
    event: ExecutionEvent,
) -> TransitionStep {
    let expected = match transition(plan, snapshot.as_ref(), event.clone()) {
        Ok(value) => TransitionExpectation::from(value),
        Err(error) => TransitionExpectation::Error {
            error: ExpectedEngineError {
                code: error.code().to_owned(),
                message: error.to_string(),
            },
        },
    };
    TransitionStep {
        snapshot,
        event,
        expected,
    }
}
