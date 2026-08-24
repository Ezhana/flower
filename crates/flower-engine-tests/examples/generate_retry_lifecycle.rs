use std::{collections::BTreeSet, env, fs, path::PathBuf, str::FromStr};

use flower_engine::{
    AttemptFailure, AttemptNumber, ExecutionEvent, ExecutionSnapshot, ExecutionState, Payload,
    transition,
};
use flower_engine::{
    BackoffPolicy, EdgeId, ExecutionId, FailureCode, NodeActivationId, NodeKind, RetryPolicy,
    TimerId, WorkflowId,
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
        .expect("usage: generate_retry_lifecycle <output-file>");
    let retryable_code = FailureCode::new("worker.transient").unwrap();
    let definition = WorkflowDefinition {
        id: WorkflowId::new("retry-lifecycle").unwrap(),
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
                    retryable_failure_codes: BTreeSet::from([retryable_code.clone()]),
                    backoff: BackoffPolicy::Exponential {
                        initial_delay_ms: 10,
                        multiplier: 3,
                        maximum_delay_ms: 25,
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
        execution_id: id("execution-retry"),
        plan_reference: plan.reference(),
        input: payload("input"),
    };
    let started_transition = transition(&plan, None, started.clone()).unwrap();
    let first_failure = failure_event(&started_transition.snapshot, retryable_code.clone());
    let first_wait = transition(
        &plan,
        Some(&started_transition.snapshot),
        first_failure.clone(),
    )
    .unwrap();
    let first_timer_fired = timer_event(&first_wait.snapshot);
    let second_attempt =
        transition(&plan, Some(&first_wait.snapshot), first_timer_fired.clone()).unwrap();
    let second_failure = failure_event(&second_attempt.snapshot, retryable_code.clone());
    let second_wait = transition(
        &plan,
        Some(&second_attempt.snapshot),
        second_failure.clone(),
    )
    .unwrap();
    let second_timer_fired = timer_event(&second_wait.snapshot);
    let third_attempt = transition(
        &plan,
        Some(&second_wait.snapshot),
        second_timer_fired.clone(),
    )
    .unwrap();
    let exhausted_failure = failure_event(&third_attempt.snapshot, retryable_code);
    let non_retryable_failure = failure_event(
        &started_transition.snapshot,
        FailureCode::new("worker.permanent").unwrap(),
    );

    let mut steps = vec![
        step(&plan, None, started),
        step(
            &plan,
            Some(started_transition.snapshot.clone()),
            first_failure,
        ),
        step(
            &plan,
            Some(first_wait.snapshot.clone()),
            first_timer_fired.clone(),
        ),
        step(&plan, Some(second_attempt.snapshot.clone()), second_failure),
        step(
            &plan,
            Some(second_wait.snapshot.clone()),
            second_timer_fired,
        ),
        step(&plan, Some(third_attempt.snapshot), exhausted_failure),
        step(
            &plan,
            Some(started_transition.snapshot),
            non_retryable_failure,
        ),
    ];

    let mut wrong_execution = first_timer_fired.clone();
    if let ExecutionEvent::TimerFired { execution_id, .. } = &mut wrong_execution {
        *execution_id = ExecutionId::new("wrong-execution").unwrap();
    }
    steps.push(step(
        &plan,
        Some(first_wait.snapshot.clone()),
        wrong_execution,
    ));

    let mut stale_revision = first_timer_fired.clone();
    if let ExecutionEvent::TimerFired {
        expected_revision, ..
    } = &mut stale_revision
    {
        expected_revision.0 -= 1;
    }
    steps.push(step(
        &plan,
        Some(first_wait.snapshot.clone()),
        stale_revision,
    ));

    let mut wrong_timer = first_timer_fired.clone();
    if let ExecutionEvent::TimerFired { timer_id, .. } = &mut wrong_timer {
        *timer_id = TimerId::new("wrong-timer").unwrap();
    }
    steps.push(step(&plan, Some(first_wait.snapshot.clone()), wrong_timer));

    let mut wrong_activation = first_timer_fired.clone();
    if let ExecutionEvent::TimerFired { activation_id, .. } = &mut wrong_activation {
        *activation_id = NodeActivationId::new("wrong-activation").unwrap();
    }
    steps.push(step(
        &plan,
        Some(first_wait.snapshot.clone()),
        wrong_activation,
    ));

    let mut wrong_attempt_number = first_timer_fired;
    if let ExecutionEvent::TimerFired {
        next_attempt_number,
        ..
    } = &mut wrong_attempt_number
    {
        *next_attempt_number = AttemptNumber::new(3).unwrap();
    }
    steps.push(step(&plan, Some(first_wait.snapshot), wrong_attempt_number));

    let fixture = EngineFixture {
        schema_version: "flower.engine-fixture/v1".to_owned(),
        name: "retry-lifecycle".to_owned(),
        definition,
        expected_compile: CompileExpectation::Plan {
            plan: ExpectedPlan::from_plan(&plan),
        },
        steps,
    };
    fs::create_dir_all(output.parent().expect("output has a parent")).unwrap();
    fs::write(output, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();
}

fn failure_event(snapshot: &ExecutionSnapshot, code: FailureCode) -> ExecutionEvent {
    let ExecutionState::AwaitingAttempt {
        activation,
        attempt,
    } = &snapshot.state
    else {
        panic!("expected awaiting attempt")
    };
    ExecutionEvent::NodeAttemptFailed {
        execution_id: snapshot.execution_id.clone(),
        expected_revision: snapshot.revision,
        activation_id: activation.activation_id.clone(),
        attempt_id: attempt.attempt_id.clone(),
        attempt_number: attempt.attempt_number,
        effect_id: attempt.effect_id.clone(),
        node_id: activation.node_id.clone(),
        failure: AttemptFailure {
            code,
            details: Some(payload("failure-details")),
        },
    }
}

fn timer_event(snapshot: &ExecutionSnapshot) -> ExecutionEvent {
    let ExecutionState::WaitingForRetry {
        activation, timer, ..
    } = &snapshot.state
    else {
        panic!("expected waiting for retry")
    };
    ExecutionEvent::TimerFired {
        execution_id: snapshot.execution_id.clone(),
        expected_revision: snapshot.revision,
        timer_id: timer.timer_id.clone(),
        activation_id: activation.activation_id.clone(),
        next_attempt_number: timer.next_attempt_number,
    }
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
