use std::{collections::BTreeMap, sync::Mutex};

use flower_host::{
    ClaimPendingEffectsRequest, ClaimedEffect, CommitError, CommitOutcome, EffectClaim,
    EffectClaimError, ExecutionCommit, ExecutionCommitParts, ExecutionHead, ExecutionStore,
    LeaseInstant, OutboxEffect, OutboxEffectStatus, StoreError, StoredExecution,
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
        let mut new_effect_ids = std::collections::BTreeSet::new();
        for effect in &commit.transition().effects {
            let effect_id = effect.effect_id();
            let already_stored = executions.values().any(|stored| {
                stored
                    .outbox
                    .iter()
                    .any(|existing| existing.effect.effect_id() == effect_id)
            });
            if already_stored || !new_effect_ids.insert(effect_id) {
                return Err(CommitError::EffectIdentityConflict {
                    effect_id: effect_id.clone(),
                });
            }
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
            .extend(transition.effects.into_iter().map(|effect| OutboxEffect {
                effect,
                status: OutboxEffectStatus::Pending,
            }));
        Ok(CommitOutcome::Committed)
    }

    async fn claim_pending_effects(
        &self,
        execution_id: &ExecutionId,
        request: &ClaimPendingEffectsRequest,
    ) -> Result<Vec<ClaimedEffect>, EffectClaimError> {
        let mut executions = self.executions.lock().map_err(poisoned)?;
        let stored = executions.get_mut(execution_id).ok_or_else(|| {
            EffectClaimError::ExecutionNotFound {
                execution_id: execution_id.clone(),
            }
        })?;
        let claim = EffectClaim {
            claim_id: request.claim_id.clone(),
            owner_id: request.owner_id.clone(),
            lease_until: request.lease_until,
        };
        let mut claimed = Vec::new();
        for outbox_effect in &mut stored.outbox {
            let available = match &outbox_effect.status {
                OutboxEffectStatus::Pending => true,
                OutboxEffectStatus::Claimed { claim } => claim.lease_until <= request.now,
                OutboxEffectStatus::Confirmed => false,
            };
            if available {
                outbox_effect.status = OutboxEffectStatus::Claimed {
                    claim: claim.clone(),
                };
                claimed.push(ClaimedEffect {
                    effect: outbox_effect.effect.clone(),
                    claim: claim.clone(),
                });
                if claimed.len() == request.maximum_count {
                    break;
                }
            }
        }
        Ok(claimed)
    }

    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
        now: LeaseInstant,
    ) -> Result<(), EffectClaimError> {
        let mut executions = self.executions.lock().map_err(poisoned)?;
        let stored = executions.get_mut(execution_id).ok_or_else(|| {
            EffectClaimError::ExecutionNotFound {
                execution_id: execution_id.clone(),
            }
        })?;
        let effect = stored
            .outbox
            .iter_mut()
            .find(|pending| pending.effect.effect_id() == effect_id)
            .ok_or_else(|| EffectClaimError::EffectNotFound {
                effect_id: effect_id.clone(),
            })?;
        let OutboxEffectStatus::Claimed {
            claim: stored_claim,
        } = &effect.status
        else {
            return Err(EffectClaimError::EffectNotClaimed {
                effect_id: effect_id.clone(),
            });
        };
        if stored_claim != claim {
            return Err(EffectClaimError::ClaimIdentityMismatch {
                effect_id: effect_id.clone(),
            });
        }
        if now >= stored_claim.lease_until {
            return Err(EffectClaimError::ClaimExpired {
                effect_id: effect_id.clone(),
                lease_until: stored_claim.lease_until,
                now,
            });
        }
        effect.status = OutboxEffectStatus::Confirmed;
        Ok(())
    }

    async fn release_effect_claim(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
    ) -> Result<(), EffectClaimError> {
        let mut executions = self.executions.lock().map_err(poisoned)?;
        let stored = executions.get_mut(execution_id).ok_or_else(|| {
            EffectClaimError::ExecutionNotFound {
                execution_id: execution_id.clone(),
            }
        })?;
        let effect = stored
            .outbox
            .iter_mut()
            .find(|pending| pending.effect.effect_id() == effect_id)
            .ok_or_else(|| EffectClaimError::EffectNotFound {
                effect_id: effect_id.clone(),
            })?;
        let OutboxEffectStatus::Claimed {
            claim: stored_claim,
        } = &effect.status
        else {
            return Err(EffectClaimError::EffectNotClaimed {
                effect_id: effect_id.clone(),
            });
        };
        if stored_claim != claim {
            return Err(EffectClaimError::ClaimIdentityMismatch {
                effect_id: effect_id.clone(),
            });
        }
        effect.status = OutboxEffectStatus::Pending;
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
        collections::BTreeSet,
        future::Future,
        pin::pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
        },
        task::{Context, Poll, Waker},
    };

    use flower_compiler::{EdgeDefinition, NodeDefinition, WorkflowDefinition, compile};
    use flower_host::{
        ClaimId, ClaimPendingEffectsRequest, CommitError, CommitOutcome, DispatchRequest,
        DispatcherId, EffectClaim, EffectClaimError, EffectDispatcher, EventHandlingOutcome,
        ExecutionCommit, ExecutionStore, LeaseClock, LeaseDuration, LeaseInstant,
        OutboxEffectStatus, WorkflowHost, replay,
    };
    use flower_kernel::{
        AttemptFailure, ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionSnapshot,
        ExecutionState, Payload, Transition, TransitionError, transition,
    };
    use flower_plan::{
        BackoffPolicy, EdgeId, EffectId, EventId, ExecutableWorkflowPlan, ExecutionId, FailureCode,
        NodeKind, RetryPolicy, WorkflowId,
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

    #[derive(Clone, Default)]
    struct ManualLeaseClock {
        milliseconds: Arc<AtomicU64>,
    }

    impl ManualLeaseClock {
        fn new(milliseconds: u64) -> Self {
            Self {
                milliseconds: Arc::new(AtomicU64::new(milliseconds)),
            }
        }

        fn set(&self, milliseconds: u64) {
            self.milliseconds.store(milliseconds, Ordering::SeqCst);
        }
    }

    impl LeaseClock for ManualLeaseClock {
        fn now(&self) -> LeaseInstant {
            LeaseInstant(self.milliseconds.load(Ordering::SeqCst))
        }
    }

    fn dispatch_request(claim_id: &str) -> DispatchRequest {
        DispatchRequest::new(
            ClaimId::new(claim_id).unwrap(),
            DispatcherId::new("dispatcher").unwrap(),
            LeaseDuration::new(100).unwrap(),
            100,
        )
        .unwrap()
    }
    fn plan() -> ExecutableWorkflowPlan {
        compile(WorkflowDefinition {
            id: WorkflowId::new("flow").unwrap(),
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
    fn started() -> ExecutionEvent {
        ExecutionEvent::ExecutionStarted {
            event_id: EventId::new("event-1").unwrap(),
            execution_id: ExecutionId::new("execution").unwrap(),
            plan_reference: plan().reference(),
            input: payload("input"),
        }
    }

    fn retry_plan() -> ExecutableWorkflowPlan {
        compile(WorkflowDefinition {
            id: WorkflowId::new("retry-flow").unwrap(),
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
                        max_attempts: 2,
                        retryable_failure_codes: BTreeSet::from([id("worker.transient")]),
                        backoff: BackoffPolicy::Fixed { delay_ms: 50 },
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

    fn retry_started(plan: &ExecutableWorkflowPlan) -> ExecutionEvent {
        ExecutionEvent::ExecutionStarted {
            event_id: id("retry-event-started"),
            execution_id: id("retry-execution"),
            plan_reference: plan.reference(),
            input: payload("input"),
        }
    }

    fn succeeded(snapshot: &ExecutionSnapshot, event_id: &str) -> ExecutionEvent {
        let ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } = &snapshot.state
        else {
            panic!("expected pending attempt")
        };
        ExecutionEvent::NodeAttemptSucceeded {
            event_id: id(event_id),
            execution_id: snapshot.execution_id.clone(),
            expected_revision: snapshot.revision,
            activation_id: activation.activation_id.clone(),
            attempt_id: attempt.attempt_id.clone(),
            attempt_number: attempt.attempt_number,
            effect_id: attempt.effect_id.clone(),
            node_id: activation.node_id.clone(),
            output: payload("done"),
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
        assert_eq!(stored.outbox[0].status, OutboxEffectStatus::Pending);
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
                    retry_policy: None,
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
        let completion = succeeded(&first.snapshot, "event-2");
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
        let competing_completion = succeeded(&first.snapshot, "event-3");
        let competing_transition =
            transition(&plan, Some(&first.snapshot), competing_completion.clone()).unwrap();
        let stored_before_conflict = block_on(store.load(&id("execution"))).unwrap().unwrap();
        assert!(matches!(
            block_on(store.commit(commit(
                first.snapshot.revision,
                competing_completion,
                competing_transition,
            ))),
            Err(CommitError::Conflict { .. })
        ));
        assert_eq!(
            block_on(store.load(&id("execution"))).unwrap().unwrap(),
            stored_before_conflict,
            "a stale commit must not partially change head, snapshot, events, or outbox"
        );
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

        let recovered_host = WorkflowHost::with_clock(store.clone(), ManualLeaseClock::new(1_000));
        let dispatcher = RecordingDispatcher::default();
        block_on(recovered_host.dispatch_pending(
            &id("execution"),
            &dispatch_request("claim-first"),
            &dispatcher,
        ))
        .unwrap();
        block_on(recovered_host.dispatch_pending(
            &id("execution"),
            &dispatch_request("claim-second"),
            &dispatcher,
        ))
        .unwrap();
        assert_eq!(dispatcher.effect_ids.lock().unwrap().len(), 1);

        let stored = block_on(store.load(&id("execution"))).unwrap().unwrap();
        assert_eq!(stored.outbox[0].status, OutboxEffectStatus::Confirmed);
        let completion = succeeded(&stored.snapshot, "event-2");
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
        let claim = EffectClaim {
            claim_id: ClaimId::new("claim").unwrap(),
            owner_id: DispatcherId::new("dispatcher").unwrap(),
            lease_until: LeaseInstant(100),
        };
        assert_eq!(
            block_on(store.confirm_effect_dispatched(
                &id("missing"),
                &id("effect-missing"),
                &claim,
                LeaseInstant(1),
            )),
            Err(EffectClaimError::ExecutionNotFound {
                execution_id: id("missing")
            })
        );

        let first = transition(&plan(), None, started()).unwrap();
        block_on(store.commit(commit(ExecutionRevision(0), started(), first))).unwrap();
        assert_eq!(
            block_on(store.confirm_effect_dispatched(
                &id("execution"),
                &id("effect-missing"),
                &claim,
                LeaseInstant(1),
            )),
            Err(EffectClaimError::EffectNotFound {
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
        let clock = ManualLeaseClock::new(1_000);
        let host = WorkflowHost::with_clock(store.clone(), clock.clone());
        block_on(host.handle_event(&plan(), started())).unwrap();

        let failing_dispatcher = FailingAfterRecordingDispatcher::default();
        assert!(
            block_on(host.dispatch_pending(
                &id("execution"),
                &dispatch_request("claim-before-crash"),
                &failing_dispatcher,
            ))
            .is_err()
        );
        clock.set(1_100);
        let successful_dispatcher = RecordingDispatcher::default();
        block_on(host.dispatch_pending(
            &id("execution"),
            &dispatch_request("claim-after-recovery"),
            &successful_dispatcher,
        ))
        .unwrap();

        assert_eq!(
            *failing_dispatcher.effect_ids.lock().unwrap(),
            *successful_dispatcher.effect_ids.lock().unwrap()
        );
        assert_eq!(successful_dispatcher.effect_ids.lock().unwrap().len(), 1);
        assert_eq!(
            block_on(store.load(&id("execution")))
                .unwrap()
                .unwrap()
                .outbox[0]
                .status,
            OutboxEffectStatus::Confirmed
        );
    }

    #[test]
    fn transition_without_commit_does_not_mutate_the_store() {
        let store = MemoryExecutionStore::new();
        let _transition = transition(&plan(), None, started()).unwrap();

        assert!(block_on(store.load(&id("execution"))).unwrap().is_none());
    }

    #[test]
    fn only_one_dispatcher_owns_an_effect_during_a_lease() {
        let store = MemoryExecutionStore::new();
        let initial = transition(&plan(), None, started()).unwrap();
        block_on(store.commit(commit(ExecutionRevision(0), started(), initial))).unwrap();
        let first_request = ClaimPendingEffectsRequest::new(
            ClaimId::new("claim-one").unwrap(),
            DispatcherId::new("dispatcher-one").unwrap(),
            LeaseInstant(10),
            LeaseInstant(20),
            1,
        )
        .unwrap();
        let second_request = ClaimPendingEffectsRequest::new(
            ClaimId::new("claim-two").unwrap(),
            DispatcherId::new("dispatcher-two").unwrap(),
            LeaseInstant(10),
            LeaseInstant(20),
            1,
        )
        .unwrap();

        let first_claim = block_on(store.claim_pending_effects(&id("execution"), &first_request))
            .unwrap()
            .pop()
            .unwrap();
        assert!(
            block_on(store.claim_pending_effects(&id("execution"), &second_request))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            block_on(store.confirm_effect_dispatched(
                &id("execution"),
                first_claim.effect.effect_id(),
                &EffectClaim {
                    claim_id: ClaimId::new("wrong-claim").unwrap(),
                    ..first_claim.claim.clone()
                },
                LeaseInstant(15),
            )),
            Err(EffectClaimError::ClaimIdentityMismatch {
                effect_id: first_claim.effect.effect_id().clone()
            })
        );
        assert_eq!(
            block_on(store.confirm_effect_dispatched(
                &id("execution"),
                first_claim.effect.effect_id(),
                &first_claim.claim,
                LeaseInstant(20),
            )),
            Err(EffectClaimError::ClaimExpired {
                effect_id: first_claim.effect.effect_id().clone(),
                lease_until: LeaseInstant(20),
                now: LeaseInstant(20),
            })
        );
        let reclaim_request = ClaimPendingEffectsRequest::new(
            ClaimId::new("claim-after-expiry").unwrap(),
            DispatcherId::new("dispatcher-two").unwrap(),
            LeaseInstant(20),
            LeaseInstant(30),
            1,
        )
        .unwrap();
        let reclaimed = block_on(store.claim_pending_effects(&id("execution"), &reclaim_request))
            .unwrap()
            .pop()
            .unwrap();
        assert_ne!(first_claim.claim, reclaimed.claim);
    }

    #[test]
    fn released_claim_can_be_reclaimed_and_confirmed() {
        let store = MemoryExecutionStore::new();
        let initial = transition(&plan(), None, started()).unwrap();
        block_on(store.commit(commit(ExecutionRevision(0), started(), initial))).unwrap();
        let first_request = ClaimPendingEffectsRequest::new(
            ClaimId::new("claim-one").unwrap(),
            DispatcherId::new("dispatcher-one").unwrap(),
            LeaseInstant(10),
            LeaseInstant(20),
            1,
        )
        .unwrap();
        let first_claim = block_on(store.claim_pending_effects(&id("execution"), &first_request))
            .unwrap()
            .pop()
            .unwrap();
        block_on(store.release_effect_claim(
            &id("execution"),
            first_claim.effect.effect_id(),
            &first_claim.claim,
        ))
        .unwrap();

        let second_request = ClaimPendingEffectsRequest::new(
            ClaimId::new("claim-two").unwrap(),
            DispatcherId::new("dispatcher-two").unwrap(),
            LeaseInstant(15),
            LeaseInstant(25),
            1,
        )
        .unwrap();
        let second_claim = block_on(store.claim_pending_effects(&id("execution"), &second_request))
            .unwrap()
            .pop()
            .unwrap();
        assert_ne!(first_claim.claim, second_claim.claim);
        block_on(store.confirm_effect_dispatched(
            &id("execution"),
            second_claim.effect.effect_id(),
            &second_claim.claim,
            LeaseInstant(24),
        ))
        .unwrap();

        let expired_reclaim_request = ClaimPendingEffectsRequest::new(
            ClaimId::new("claim-three").unwrap(),
            DispatcherId::new("dispatcher-three").unwrap(),
            LeaseInstant(25),
            LeaseInstant(35),
            1,
        )
        .unwrap();
        assert!(
            block_on(store.claim_pending_effects(&id("execution"), &expired_reclaim_request))
                .unwrap()
                .is_empty(),
            "confirmed effects must never be reclaimable"
        );
    }

    #[test]
    fn duplicate_timer_delivery_does_not_create_an_extra_attempt() {
        let store = Arc::new(MemoryExecutionStore::new());
        let host = WorkflowHost::new(store.clone());
        let plan = retry_plan();
        let started = retry_started(&plan);
        let EventHandlingOutcome::Committed(started_transition) =
            block_on(host.handle_event(&plan, started)).unwrap()
        else {
            panic!("start must commit")
        };
        let ExecutionState::AwaitingAttempt {
            activation,
            attempt,
        } = &started_transition.snapshot.state
        else {
            panic!("start must create attempt one")
        };
        let failed = ExecutionEvent::NodeAttemptFailed {
            event_id: id("retry-event-failed"),
            execution_id: started_transition.snapshot.execution_id.clone(),
            expected_revision: started_transition.snapshot.revision,
            activation_id: activation.activation_id.clone(),
            attempt_id: attempt.attempt_id.clone(),
            attempt_number: attempt.attempt_number,
            effect_id: attempt.effect_id.clone(),
            node_id: activation.node_id.clone(),
            failure: AttemptFailure {
                code: FailureCode::new("worker.transient").unwrap(),
                details: None,
            },
        };
        let EventHandlingOutcome::Committed(waiting_transition) =
            block_on(host.handle_event(&plan, failed)).unwrap()
        else {
            panic!("retryable failure must commit")
        };
        assert_eq!(
            replay(
                &plan,
                block_on(store.load(&id("retry-execution")))
                    .unwrap()
                    .unwrap()
                    .events
            )
            .unwrap(),
            Some(waiting_transition.snapshot.clone())
        );
        let ExecutionState::WaitingForRetry {
            activation, timer, ..
        } = &waiting_transition.snapshot.state
        else {
            panic!("retryable failure must wait for its timer")
        };
        let timer_fired = ExecutionEvent::TimerFired {
            event_id: id("retry-event-timer-fired"),
            execution_id: waiting_transition.snapshot.execution_id.clone(),
            expected_revision: waiting_transition.snapshot.revision,
            timer_id: timer.timer_id.clone(),
            activation_id: activation.activation_id.clone(),
            next_attempt_number: timer.next_attempt_number,
        };
        let EventHandlingOutcome::Committed(next_attempt) =
            block_on(host.handle_event(&plan, timer_fired.clone())).unwrap()
        else {
            panic!("first timer delivery must commit")
        };
        let ExecutionState::AwaitingAttempt { attempt, .. } = &next_attempt.snapshot.state else {
            panic!("timer must create attempt two")
        };
        assert_eq!(attempt.attempt_number.value(), 2);
        assert_eq!(
            block_on(host.handle_event(&plan, timer_fired)).unwrap(),
            EventHandlingOutcome::AlreadyCommitted
        );
        let stored = block_on(store.load(&id("retry-execution")))
            .unwrap()
            .unwrap();
        assert_eq!(stored.events.len(), 3);
        assert_eq!(stored.outbox.len(), 3);
        assert_eq!(stored.snapshot, next_attempt.snapshot);
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
