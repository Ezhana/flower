use std::{collections::BTreeMap, sync::Mutex};

use flower_host::{
    CommitError, CommitOutcome, ConfirmEffectError, ExecutionCommit, ExecutionCommitParts,
    ExecutionHead, ExecutionStore, PendingEffect, StoreError, StoredExecution,
};
use flower_plan::{EffectId, ExecutionId};

#[derive(Debug, Default)]
pub struct MemoryExecutionStore {
    executions: Mutex<BTreeMap<ExecutionId, StoredExecution>>,
}

impl MemoryExecutionStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExecutionStore for MemoryExecutionStore {
    async fn load(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<Option<StoredExecution>, StoreError> {
        Ok(self
            .executions
            .lock()
            .map_err(poisoned)?
            .get(execution_id)
            .cloned())
    }

    async fn commit(&self, commit: ExecutionCommit) -> Result<CommitOutcome, CommitError> {
        let mut executions = self.executions.lock().map_err(poisoned)?;
        if let Some(existing) = executions.get(commit.execution_id()).and_then(|stored| {
            stored
                .events
                .iter()
                .find(|existing| existing.event_id() == commit.event().event_id())
        }) {
            if existing == commit.event() {
                return Ok(CommitOutcome::AlreadyCommitted);
            }
            return Err(CommitError::EventIdentityConflict {
                event_id: commit.event().event_id().clone(),
            });
        }
        let actual_revision = executions
            .get(commit.execution_id())
            .map_or(flower_kernel::ExecutionRevision(0), |stored| {
                stored.head.revision
            });
        if actual_revision != commit.expected_revision() {
            return Err(CommitError::Conflict {
                expected: commit.expected_revision(),
                actual: actual_revision,
            });
        }
        if let Some(stored) = executions.get(commit.execution_id())
            && stored.head.plan_reference != commit.transition().snapshot.plan_reference
        {
            return Err(CommitError::PlanReferenceMismatch);
        }
        let ExecutionCommitParts {
            execution_id,
            event,
            transition,
            ..
        } = commit.into_parts();
        let head = ExecutionHead {
            plan_reference: transition.snapshot.plan_reference.clone(),
            revision: transition.snapshot.revision,
        };
        let stored = executions
            .entry(execution_id)
            .or_insert_with(|| StoredExecution {
                head: head.clone(),
                snapshot: transition.snapshot.clone(),
                events: Vec::new(),
                outbox: Vec::new(),
            });
        stored.head = head;
        stored.snapshot = transition.snapshot;
        stored.events.push(event);
        stored
            .outbox
            .extend(transition.effects.into_iter().map(|effect| PendingEffect {
                effect,
                dispatched: false,
            }));
        Ok(CommitOutcome::Committed)
    }

    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
    ) -> Result<(), ConfirmEffectError> {
        let mut executions = self.executions.lock().map_err(poisoned)?;
        let stored = executions.get_mut(execution_id).ok_or_else(|| {
            ConfirmEffectError::ExecutionNotFound {
                execution_id: execution_id.clone(),
            }
        })?;
        let effect = stored
            .outbox
            .iter_mut()
            .find(|pending| pending.effect.effect_id() == effect_id)
            .ok_or_else(|| ConfirmEffectError::EffectNotFound {
                effect_id: effect_id.clone(),
            })?;
        effect.dispatched = true;
        Ok(())
    }
}

fn poisoned<T>(_: std::sync::PoisonError<T>) -> StoreError {
    StoreError::Unavailable {
        message: "memory store lock was poisoned".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::pin,
        sync::{Arc, Mutex},
        task::{Context, Poll, Waker},
    };

    use flower_compiler::{EdgeDefinition, NodeDefinition, WorkflowDefinition, compile};
    use flower_host::{
        CommitError, CommitOutcome, ConfirmEffectError, EffectDispatcher, EventHandlingOutcome,
        ExecutionCommit, ExecutionStore, WorkflowHost, replay,
    };
    use flower_kernel::{
        ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionState, Payload, Transition,
        TransitionError, transition,
    };
    use flower_plan::{
        EdgeId, EffectId, EventId, ExecutableWorkflowPlan, ExecutionId, NodeKind, WorkflowId,
    };

    use super::*;

    fn block_on<Output>(future: impl Future<Output = Output>) -> Output {
        let mut future = pin!(future);
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }

    fn id<T: std::str::FromStr>(value: &str) -> T
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
    fn plan() -> ExecutableWorkflowPlan {
        compile(WorkflowDefinition {
            id: WorkflowId::new("flow").unwrap(),
            nodes: vec![
                NodeDefinition {
                    id: id("start"),
                    kind: NodeKind::Start,
                },
                NodeDefinition {
                    id: id("work"),
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
    fn started() -> ExecutionEvent {
        ExecutionEvent::ExecutionStarted {
            event_id: EventId::new("event-1").unwrap(),
            execution_id: ExecutionId::new("execution").unwrap(),
            plan_reference: plan().reference(),
            input: payload("input"),
        }
    }

    fn commit(
        expected_revision: ExecutionRevision,
        event: ExecutionEvent,
        transition: Transition,
    ) -> ExecutionCommit {
        ExecutionCommit::new(expected_revision, event, transition).unwrap()
    }

    #[test]
    fn atomically_commits_snapshot_event_and_outbox_and_replays() {
        let store = Arc::new(MemoryExecutionStore::new());
        let host = WorkflowHost::new(store.clone());
        let EventHandlingOutcome::Committed(transition) =
            block_on(host.handle_event(&plan(), started())).unwrap()
        else {
            panic!("a new event must be committed")
        };
        let stored = block_on(store.load(&ExecutionId::new("execution").unwrap()))
            .unwrap()
            .unwrap();
        assert_eq!(stored.snapshot, transition.snapshot);
        assert_eq!(stored.head.plan_reference, stored.snapshot.plan_reference);
        assert_eq!(stored.head.revision, stored.snapshot.revision);
        assert_eq!(stored.events, vec![started()]);
        assert_eq!(stored.outbox.len(), 1);
        assert!(!stored.outbox[0].dispatched);
        assert_eq!(
            replay(&plan(), stored.events).unwrap(),
            Some(stored.snapshot)
        );
    }

    #[test]
    fn replay_rejects_an_event_log_bound_to_another_plan() {
        let original = plan();
        let mut other_definition = WorkflowDefinition {
            id: WorkflowId::new("other-flow").unwrap(),
            nodes: original
                .nodes()
                .iter()
                .map(|node| NodeDefinition {
                    id: node.id.clone(),
                    kind: node.kind,
                })
                .collect(),
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
        };
        other_definition.nodes.reverse();
        let other = compile(other_definition).unwrap();

        assert_eq!(
            replay(&other, [started()]),
            Err(TransitionError::PlanReferenceMismatch)
        );
    }

    #[test]
    fn optimistic_commit_rejects_two_hosts_advancing_one_revision() {
        let store = MemoryExecutionStore::new();
        let plan = plan();
        let first = transition(&plan, None, started()).unwrap();
        assert_eq!(
            block_on(store.commit(commit(ExecutionRevision(0), started(), first.clone(),)))
                .unwrap(),
            CommitOutcome::Committed
        );
        let ExecutionState::AwaitingNode {
            node_id, effect_id, ..
        } = &first.snapshot.state
        else {
            panic!()
        };
        let completion = ExecutionEvent::NodeCompleted {
            event_id: id("event-2"),
            execution_id: id("execution"),
            expected_revision: first.snapshot.revision,
            effect_id: effect_id.clone(),
            node_id: node_id.clone(),
            output: payload("done"),
        };
        let next = transition(&plan, Some(&first.snapshot), completion.clone()).unwrap();
        assert_eq!(
            block_on(store.commit(commit(
                first.snapshot.revision,
                completion.clone(),
                next.clone(),
            )))
            .unwrap(),
            CommitOutcome::Committed
        );
        let competing_completion = ExecutionEvent::NodeCompleted {
            event_id: id("event-3"),
            execution_id: id("execution"),
            expected_revision: first.snapshot.revision,
            effect_id: effect_id.clone(),
            node_id: node_id.clone(),
            output: payload("done"),
        };
        let competing_transition =
            transition(&plan, Some(&first.snapshot), competing_completion.clone()).unwrap();
        assert!(matches!(
            block_on(store.commit(commit(
                first.snapshot.revision,
                competing_completion,
                competing_transition,
            ))),
            Err(CommitError::Conflict { .. })
        ));
        assert_eq!(
            block_on(store.commit(commit(first.snapshot.revision, completion, next,))).unwrap(),
            CommitOutcome::AlreadyCommitted
        );
    }

    #[test]
    fn recovers_pending_effect_and_does_not_redispatch_after_confirmation() {
        let store = Arc::new(MemoryExecutionStore::new());
        let first_host = WorkflowHost::new(store.clone());
        block_on(first_host.handle_event(&plan(), started())).unwrap();

        let recovered_host = WorkflowHost::new(store.clone());
        let dispatcher = RecordingDispatcher::default();
        block_on(recovered_host.dispatch_pending(&id("execution"), &dispatcher)).unwrap();
        block_on(recovered_host.dispatch_pending(&id("execution"), &dispatcher)).unwrap();
        assert_eq!(dispatcher.effect_ids.lock().unwrap().len(), 1);

        let stored = block_on(store.load(&id("execution"))).unwrap().unwrap();
        assert!(stored.outbox[0].dispatched);
        let ExecutionState::AwaitingNode {
            node_id, effect_id, ..
        } = &stored.snapshot.state
        else {
            panic!()
        };
        let completion = ExecutionEvent::NodeCompleted {
            event_id: id("event-2"),
            execution_id: id("execution"),
            expected_revision: stored.snapshot.revision,
            effect_id: effect_id.clone(),
            node_id: node_id.clone(),
            output: payload("done"),
        };
        let EventHandlingOutcome::Committed(completed) =
            block_on(recovered_host.handle_event(&plan(), completion.clone())).unwrap()
        else {
            panic!("a new completion must be committed")
        };
        assert!(matches!(
            completed.snapshot.state,
            ExecutionState::Completed { .. }
        ));
        assert_eq!(
            block_on(recovered_host.handle_event(&plan(), completion)).unwrap(),
            EventHandlingOutcome::AlreadyCommitted
        );
    }

    #[test]
    fn confirming_an_unknown_effect_is_a_domain_error() {
        let store = MemoryExecutionStore::new();
        assert_eq!(
            block_on(store.confirm_effect_dispatched(&id("missing"), &id("effect-missing"))),
            Err(ConfirmEffectError::ExecutionNotFound {
                execution_id: id("missing")
            })
        );

        let first = transition(&plan(), None, started()).unwrap();
        block_on(store.commit(commit(ExecutionRevision(0), started(), first))).unwrap();
        assert_eq!(
            block_on(store.confirm_effect_dispatched(&id("execution"), &id("effect-missing"),)),
            Err(ConfirmEffectError::EffectNotFound {
                effect_id: id("effect-missing")
            })
        );
    }

    #[test]
    fn event_identity_is_scoped_to_one_execution() {
        let store = Arc::new(MemoryExecutionStore::new());
        let host = WorkflowHost::new(store);
        assert!(matches!(
            block_on(host.handle_event(&plan(), started())).unwrap(),
            EventHandlingOutcome::Committed(_)
        ));

        let conflicting_event = ExecutionEvent::ExecutionStarted {
            event_id: id("event-1"),
            execution_id: id("execution"),
            plan_reference: plan().reference(),
            input: payload("different-input"),
        };
        assert!(matches!(
            block_on(host.handle_event(&plan(), conflicting_event)),
            Err(flower_host::HostError::Commit(
                CommitError::EventIdentityConflict { .. }
            ))
        ));

        let another_execution = ExecutionEvent::ExecutionStarted {
            event_id: id("event-1"),
            execution_id: id("another-execution"),
            plan_reference: plan().reference(),
            input: payload("input"),
        };
        assert!(matches!(
            block_on(host.handle_event(&plan(), another_execution)).unwrap(),
            EventHandlingOutcome::Committed(_)
        ));
    }

    #[test]
    fn dispatch_is_at_least_once_when_confirmation_is_not_recorded() {
        let store = Arc::new(MemoryExecutionStore::new());
        let host = WorkflowHost::new(store.clone());
        block_on(host.handle_event(&plan(), started())).unwrap();

        let failing_dispatcher = FailingAfterRecordingDispatcher::default();
        assert!(block_on(host.dispatch_pending(&id("execution"), &failing_dispatcher)).is_err());
        let successful_dispatcher = RecordingDispatcher::default();
        block_on(host.dispatch_pending(&id("execution"), &successful_dispatcher)).unwrap();

        assert_eq!(
            *failing_dispatcher.effect_ids.lock().unwrap(),
            *successful_dispatcher.effect_ids.lock().unwrap()
        );
        assert_eq!(successful_dispatcher.effect_ids.lock().unwrap().len(), 1);
        assert!(
            block_on(store.load(&id("execution")))
                .unwrap()
                .unwrap()
                .outbox[0]
                .dispatched
        );
    }

    #[derive(Default)]
    struct RecordingDispatcher {
        effect_ids: Mutex<Vec<EffectId>>,
    }

    #[derive(Default)]
    struct FailingAfterRecordingDispatcher {
        effect_ids: Mutex<Vec<EffectId>>,
    }

    impl EffectDispatcher for FailingAfterRecordingDispatcher {
        type Error = &'static str;

        async fn dispatch(&self, effect: &ExecutionEffect) -> Result<(), Self::Error> {
            self.effect_ids
                .lock()
                .unwrap()
                .push(effect.effect_id().clone());
            Err("executor exited before outbox confirmation")
        }
    }

    impl EffectDispatcher for RecordingDispatcher {
        type Error = std::convert::Infallible;

        async fn dispatch(&self, effect: &ExecutionEffect) -> Result<(), Self::Error> {
            self.effect_ids
                .lock()
                .unwrap()
                .push(effect.effect_id().clone());
            Ok(())
        }
    }
}
