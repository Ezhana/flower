use std::{collections::BTreeMap, sync::Mutex};

use flower_host::{CommitError, ExecutionStore, PendingEffect, StoreError, StoredExecution};
use flower_kernel::{ExecutionEvent, ExecutionRevision, Transition};
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

    async fn commit(
        &self,
        execution_id: &ExecutionId,
        expected_revision: ExecutionRevision,
        event: ExecutionEvent,
        transition: Transition,
    ) -> Result<(), CommitError> {
        if execution_id != &transition.snapshot.execution_id || execution_id != event.execution_id()
        {
            return Err(CommitError::ExecutionIdMismatch);
        }
        let mut executions = self.executions.lock().map_err(poisoned)?;
        let actual_revision = executions
            .get(execution_id)
            .map_or(ExecutionRevision(0), |stored| stored.snapshot.revision);
        if actual_revision != expected_revision {
            return Err(CommitError::Conflict {
                expected: expected_revision,
                actual: actual_revision,
            });
        }
        if executions.get(execution_id).is_some_and(|stored| {
            stored
                .events
                .iter()
                .any(|existing| existing.event_id() == event.event_id())
        }) {
            return Err(CommitError::DuplicateEvent {
                event_id: event.event_id().clone(),
            });
        }
        let stored = executions
            .entry(execution_id.clone())
            .or_insert_with(|| StoredExecution {
                snapshot: transition.snapshot.clone(),
                events: Vec::new(),
                outbox: Vec::new(),
            });
        stored.snapshot = transition.snapshot;
        stored.events.push(event);
        stored
            .outbox
            .extend(transition.effects.into_iter().map(|effect| PendingEffect {
                effect,
                dispatched: false,
            }));
        Ok(())
    }

    async fn mark_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
    ) -> Result<(), StoreError> {
        let mut executions = self.executions.lock().map_err(poisoned)?;
        if let Some(effect) = executions.get_mut(execution_id).and_then(|stored| {
            stored
                .outbox
                .iter_mut()
                .find(|pending| pending.effect.effect_id() == effect_id)
        }) {
            effect.dispatched = true;
        }
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
    use flower_host::{CommitError, EffectDispatcher, ExecutionStore, WorkflowHost, replay};
    use flower_kernel::{
        ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionState, Payload,
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

    #[test]
    fn atomically_commits_snapshot_event_and_outbox_and_replays() {
        let store = Arc::new(MemoryExecutionStore::new());
        let host = WorkflowHost::new(store.clone());
        let transition = block_on(host.handle(&plan(), started())).unwrap();
        let stored = block_on(store.load(&ExecutionId::new("execution").unwrap()))
            .unwrap()
            .unwrap();
        assert_eq!(stored.snapshot, transition.snapshot);
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
        block_on(store.commit(
            &id("execution"),
            ExecutionRevision(0),
            started(),
            first.clone(),
        ))
        .unwrap();
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
        block_on(store.commit(
            &id("execution"),
            first.snapshot.revision,
            completion.clone(),
            next.clone(),
        ))
        .unwrap();
        assert!(matches!(
            block_on(store.commit(&id("execution"), first.snapshot.revision, completion, next)),
            Err(CommitError::Conflict { .. })
        ));
    }

    #[test]
    fn recovers_pending_effect_and_does_not_redispatch_after_confirmation() {
        let store = Arc::new(MemoryExecutionStore::new());
        let first_host = WorkflowHost::new(store.clone());
        block_on(first_host.handle(&plan(), started())).unwrap();

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
        let completed = block_on(recovered_host.handle(&plan(), completion.clone())).unwrap();
        assert!(matches!(
            completed.snapshot.state,
            ExecutionState::Completed { .. }
        ));
        assert!(matches!(
            block_on(recovered_host.handle(&plan(), completion)),
            Err(flower_host::HostError::Commit(
                CommitError::DuplicateEvent { .. }
            ))
        ));
    }

    #[derive(Default)]
    struct RecordingDispatcher {
        effect_ids: Mutex<Vec<EffectId>>,
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
