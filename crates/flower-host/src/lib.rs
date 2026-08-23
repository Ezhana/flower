#![allow(async_fn_in_trait)]

use flower_kernel::{
    ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionSnapshot, Transition,
    TransitionError, transition, validate_snapshot,
};
use flower_plan::{EffectId, EventId, ExecutableWorkflowPlan, ExecutionId, PlanReference};
use serde::{Deserialize, Deserializer, Serialize, de};
use std::{collections::BTreeSet, sync::Arc};
use thiserror::Error;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ClaimId(String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct DispatcherId(String);

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DispatchIdentityError {
    #[error("dispatch identity cannot be empty")]
    Empty,
    #[error("dispatch identity contains unsupported characters")]
    UnsupportedCharacters,
}

macro_rules! dispatch_identity {
    ($name:ident) => {
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DispatchIdentityError> {
                let value = value.into();
                if value.is_empty() {
                    return Err(DispatchIdentityError::Empty);
                }
                if !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                {
                    return Err(DispatchIdentityError::UnsupportedCharacters);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
            }
        }
    };
}

dispatch_identity!(ClaimId);
dispatch_identity!(DispatcherId);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LeaseInstant(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseDuration(u64);

impl LeaseDuration {
    pub fn new(milliseconds: u64) -> Option<Self> {
        (milliseconds > 0).then_some(Self(milliseconds))
    }

    pub fn milliseconds(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectClaim {
    pub claim_id: ClaimId,
    pub owner_id: DispatcherId,
    pub lease_until: LeaseInstant,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case", deny_unknown_fields)]
pub enum OutboxEffectStatus {
    Pending,
    Claimed { claim: EffectClaim },
    Confirmed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxEffect {
    pub effect: ExecutionEffect,
    pub status: OutboxEffectStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimedEffect {
    pub effect: ExecutionEffect,
    pub claim: EffectClaim,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimPendingEffectsRequest {
    pub claim_id: ClaimId,
    pub owner_id: DispatcherId,
    pub now: LeaseInstant,
    pub lease_until: LeaseInstant,
    pub maximum_count: usize,
}

impl ClaimPendingEffectsRequest {
    pub fn new(
        claim_id: ClaimId,
        owner_id: DispatcherId,
        now: LeaseInstant,
        lease_until: LeaseInstant,
        maximum_count: usize,
    ) -> Result<Self, InvalidClaimRequest> {
        if lease_until <= now {
            return Err(InvalidClaimRequest::NonFutureLease);
        }
        if maximum_count == 0 {
            return Err(InvalidClaimRequest::ZeroMaximumCount);
        }
        Ok(Self {
            claim_id,
            owner_id,
            now,
            lease_until,
            maximum_count,
        })
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum InvalidClaimRequest {
    #[error("claim lease must end after the claim time")]
    NonFutureLease,
    #[error("claim maximum count must be non-zero")]
    ZeroMaximumCount,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionHead {
    pub plan_reference: PlanReference,
    pub revision: ExecutionRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredExecution {
    pub head: ExecutionHead,
    pub snapshot: ExecutionSnapshot,
    pub events: Vec<ExecutionEvent>,
    pub outbox: Vec<OutboxEffect>,
}

impl StoredExecution {
    pub fn validate_consistency(&self, execution_id: &ExecutionId) -> Result<(), StoreError> {
        let first_event_matches = matches!(
            self.events.first(),
            Some(ExecutionEvent::ExecutionStarted {
                execution_id: started_execution_id,
                plan_reference,
                ..
            }) if started_execution_id == execution_id && plan_reference == &self.head.plan_reference
        );
        let mut event_ids = BTreeSet::new();
        let events_are_consistent = self.events.iter().all(|event| {
            event.execution_id() == execution_id && event_ids.insert(event.event_id())
        });
        let mut effect_ids = BTreeSet::new();
        let outbox_is_consistent = self
            .outbox
            .iter()
            .all(|entry| effect_ids.insert(entry.effect.effect_id()));
        let event_count = u64::try_from(self.events.len()).ok();
        if self.snapshot.execution_id != *execution_id
            || self.snapshot.plan_reference != self.head.plan_reference
            || self.snapshot.revision != self.head.revision
            || event_count != Some(self.head.revision.0)
            || !first_event_matches
            || !events_are_consistent
            || !outbox_is_consistent
        {
            return Err(StoreError::InconsistentExecution {
                execution_id: execution_id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionCommit {
    execution_id: ExecutionId,
    expected_revision: ExecutionRevision,
    event: ExecutionEvent,
    transition: Transition,
}

pub struct ExecutionCommitParts {
    pub execution_id: ExecutionId,
    pub expected_revision: ExecutionRevision,
    pub event: ExecutionEvent,
    pub transition: Transition,
}

impl ExecutionCommit {
    pub fn new(
        expected_revision: ExecutionRevision,
        event: ExecutionEvent,
        transition: Transition,
    ) -> Result<Self, CommitError> {
        let execution_id = event.execution_id().clone();
        if execution_id != transition.snapshot.execution_id {
            return Err(CommitError::ExecutionIdMismatch);
        }
        let expected_next_revision = expected_revision
            .0
            .checked_add(1)
            .map(ExecutionRevision)
            .ok_or(CommitError::InvalidRevision {
                expected_previous: expected_revision,
                actual: transition.snapshot.revision,
            })?;
        if transition.snapshot.revision != expected_next_revision {
            return Err(CommitError::InvalidRevision {
                expected_previous: expected_revision,
                actual: transition.snapshot.revision,
            });
        }
        match (&event, expected_revision.0) {
            (ExecutionEvent::ExecutionStarted { .. }, 0)
            | (ExecutionEvent::NodeAttemptSucceeded { .. }, 1..)
            | (ExecutionEvent::NodeAttemptFailed { .. }, 1..)
            | (ExecutionEvent::TimerFired { .. }, 1..) => {}
            _ => return Err(CommitError::InvalidEventSequence),
        }
        if let ExecutionEvent::ExecutionStarted { plan_reference, .. } = &event
            && plan_reference != &transition.snapshot.plan_reference
        {
            return Err(CommitError::PlanReferenceMismatch);
        }
        Ok(Self {
            execution_id,
            expected_revision,
            event,
            transition,
        })
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn expected_revision(&self) -> ExecutionRevision {
        self.expected_revision
    }

    pub fn event(&self) -> &ExecutionEvent {
        &self.event
    }

    pub fn transition(&self) -> &Transition {
        &self.transition
    }

    pub fn into_parts(self) -> ExecutionCommitParts {
        ExecutionCommitParts {
            execution_id: self.execution_id,
            expected_revision: self.expected_revision,
            event: self.event,
            transition: self.transition,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    Committed,
    AlreadyCommitted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventHandlingOutcome {
    Committed(Box<Transition>),
    AlreadyCommitted,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StoreError {
    #[error("execution store is unavailable: {message}")]
    Unavailable { message: String },
    #[error("stored execution `{execution_id}` violates the execution log invariants")]
    InconsistentExecution { execution_id: ExecutionId },
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CommitError {
    #[error("execution revision conflict: expected {expected:?}, actual {actual:?}")]
    Conflict {
        expected: ExecutionRevision,
        actual: ExecutionRevision,
    },
    #[error("event id `{event_id}` is already bound to a different event in this execution")]
    EventIdentityConflict { event_id: EventId },
    #[error("effect id `{effect_id}` is already bound in the global outbox")]
    EffectIdentityConflict { effect_id: EffectId },
    #[error("commit execution id does not match transition")]
    ExecutionIdMismatch,
    #[error("commit must advance exactly once from {expected_previous:?}, got {actual:?}")]
    InvalidRevision {
        expected_previous: ExecutionRevision,
        actual: ExecutionRevision,
    },
    #[error("commit plan reference is inconsistent")]
    PlanReferenceMismatch,
    #[error("commit event is not legal at the expected revision")]
    InvalidEventSequence,
    #[error(transparent)]
    Store(#[from] StoreError),
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum EffectClaimError {
    #[error("execution `{execution_id}` does not exist")]
    ExecutionNotFound { execution_id: ExecutionId },
    #[error("effect `{effect_id}` does not exist in the execution outbox")]
    EffectNotFound { effect_id: EffectId },
    #[error("effect `{effect_id}` is not currently claimed")]
    EffectNotClaimed { effect_id: EffectId },
    #[error("claim identity does not own effect `{effect_id}`")]
    ClaimIdentityMismatch { effect_id: EffectId },
    #[error("claim for effect `{effect_id}` expired at {lease_until:?}; current time is {now:?}")]
    ClaimExpired {
        effect_id: EffectId,
        lease_until: LeaseInstant,
        now: LeaseInstant,
    },
    #[error(transparent)]
    InvalidRequest(#[from] InvalidClaimRequest),
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub trait ExecutionStore: Send + Sync {
    async fn load(&self, execution_id: &ExecutionId)
    -> Result<Option<StoredExecution>, StoreError>;
    async fn commit(&self, commit: ExecutionCommit) -> Result<CommitOutcome, CommitError>;
    async fn claim_pending_effects(
        &self,
        execution_id: &ExecutionId,
        request: &ClaimPendingEffectsRequest,
    ) -> Result<Vec<ClaimedEffect>, EffectClaimError>;
    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
        now: LeaseInstant,
    ) -> Result<(), EffectClaimError>;
    async fn release_effect_claim(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
    ) -> Result<(), EffectClaimError>;
}

impl<Store: ExecutionStore + ?Sized> ExecutionStore for Arc<Store> {
    async fn load(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<Option<StoredExecution>, StoreError> {
        (**self).load(execution_id).await
    }
    async fn commit(&self, commit: ExecutionCommit) -> Result<CommitOutcome, CommitError> {
        (**self).commit(commit).await
    }
    async fn claim_pending_effects(
        &self,
        execution_id: &ExecutionId,
        request: &ClaimPendingEffectsRequest,
    ) -> Result<Vec<ClaimedEffect>, EffectClaimError> {
        (**self).claim_pending_effects(execution_id, request).await
    }
    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
        now: LeaseInstant,
    ) -> Result<(), EffectClaimError> {
        (**self)
            .confirm_effect_dispatched(execution_id, effect_id, claim, now)
            .await
    }
    async fn release_effect_claim(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
    ) -> Result<(), EffectClaimError> {
        (**self)
            .release_effect_claim(execution_id, effect_id, claim)
            .await
    }
}

pub trait EffectDispatcher: Send + Sync {
    type Error;
    async fn dispatch(&self, effect: &ExecutionEffect) -> Result<(), Self::Error>;
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error(transparent)]
    Transition(#[from] TransitionError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Commit(#[from] CommitError),
}

pub trait LeaseClock: Send + Sync {
    fn now(&self) -> LeaseInstant;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemLeaseClock;

impl LeaseClock for SystemLeaseClock {
    fn now(&self) -> LeaseInstant {
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        LeaseInstant(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchRequest {
    pub claim_id: ClaimId,
    pub owner_id: DispatcherId,
    pub lease_duration: LeaseDuration,
    pub maximum_count: usize,
}

impl DispatchRequest {
    pub fn new(
        claim_id: ClaimId,
        owner_id: DispatcherId,
        lease_duration: LeaseDuration,
        maximum_count: usize,
    ) -> Result<Self, InvalidClaimRequest> {
        if maximum_count == 0 {
            return Err(InvalidClaimRequest::ZeroMaximumCount);
        }
        Ok(Self {
            claim_id,
            owner_id,
            lease_duration,
            maximum_count,
        })
    }
}

pub struct WorkflowHost<Store, Clock = SystemLeaseClock> {
    store: Store,
    clock: Clock,
}

impl<Store: ExecutionStore> WorkflowHost<Store, SystemLeaseClock> {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            clock: SystemLeaseClock,
        }
    }
}

impl<Store: ExecutionStore, Clock: LeaseClock> WorkflowHost<Store, Clock> {
    pub fn with_clock(store: Store, clock: Clock) -> Self {
        Self { store, clock }
    }
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub async fn handle_event(
        &self,
        plan: &ExecutableWorkflowPlan,
        event: ExecutionEvent,
    ) -> Result<EventHandlingOutcome, HostError> {
        let execution_id = event.execution_id().clone();
        let stored = self.store.load(&execution_id).await?;
        if let Some(stored) = &stored {
            stored.validate_consistency(&execution_id)?;
            validate_snapshot(plan, &stored.snapshot)?;
        }
        if let Some(existing) = stored.as_ref().and_then(|stored| {
            stored
                .events
                .iter()
                .find(|existing| existing.event_id() == event.event_id())
        }) {
            if existing == &event {
                return Ok(EventHandlingOutcome::AlreadyCommitted);
            }
            return Err(CommitError::EventIdentityConflict {
                event_id: event.event_id().clone(),
            }
            .into());
        }
        let snapshot = stored.as_ref().map(|value| &value.snapshot);
        let expected_revision = snapshot.map_or(ExecutionRevision(0), |value| value.revision);
        let next = transition(plan, snapshot, event.clone())?;
        let commit = ExecutionCommit::new(expected_revision, event, next.clone())?;
        match self.store.commit(commit).await? {
            CommitOutcome::Committed => Ok(EventHandlingOutcome::Committed(Box::new(next))),
            CommitOutcome::AlreadyCommitted => Ok(EventHandlingOutcome::AlreadyCommitted),
        }
    }

    pub async fn dispatch_pending<Dispatcher: EffectDispatcher>(
        &self,
        execution_id: &ExecutionId,
        request: &DispatchRequest,
        dispatcher: &Dispatcher,
    ) -> Result<(), DispatchError<Dispatcher::Error>> {
        let now = self.clock.now();
        let lease_until = LeaseInstant(
            now.0
                .checked_add(request.lease_duration.milliseconds())
                .ok_or(DispatchError::LeaseOverflow)?,
        );
        let claim_request = ClaimPendingEffectsRequest::new(
            request.claim_id.clone(),
            request.owner_id.clone(),
            now,
            lease_until,
            request.maximum_count,
        )
        .map_err(DispatchError::InvalidRequest)?;
        let claimed = self
            .store
            .claim_pending_effects(execution_id, &claim_request)
            .await
            .map_err(DispatchError::Claim)?;
        for claimed_effect in claimed {
            dispatcher
                .dispatch(&claimed_effect.effect)
                .await
                .map_err(DispatchError::Dispatcher)?;
            self.store
                .confirm_effect_dispatched(
                    execution_id,
                    claimed_effect.effect.effect_id(),
                    &claimed_effect.claim,
                    self.clock.now(),
                )
                .await
                .map_err(DispatchError::Confirm)?;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum DispatchError<DispatcherError> {
    #[error("effect dispatcher failed")]
    Dispatcher(DispatcherError),
    #[error("dispatch lease instant overflowed")]
    LeaseOverflow,
    #[error(transparent)]
    InvalidRequest(InvalidClaimRequest),
    #[error(transparent)]
    Claim(EffectClaimError),
    #[error(transparent)]
    Confirm(EffectClaimError),
}

pub fn replay(
    plan: &ExecutableWorkflowPlan,
    events: impl IntoIterator<Item = ExecutionEvent>,
) -> Result<Option<ExecutionSnapshot>, TransitionError> {
    let mut events = events.into_iter();
    let Some(first_event) = events.next() else {
        return Ok(None);
    };
    match &first_event {
        ExecutionEvent::ExecutionStarted { plan_reference, .. }
            if plan_reference == &plan.reference() => {}
        ExecutionEvent::ExecutionStarted { .. } => {
            return Err(TransitionError::PlanReferenceMismatch);
        }
        ExecutionEvent::NodeAttemptSucceeded { .. } | ExecutionEvent::NodeAttemptFailed { .. } => {
            return Err(TransitionError::ExecutionNotStarted);
        }
        ExecutionEvent::TimerFired { .. } => return Err(TransitionError::ExecutionNotStarted),
    }

    let mut snapshot = Some(transition(plan, None, first_event)?.snapshot);
    for event in events {
        snapshot = Some(transition(plan, snapshot.as_ref(), event)?.snapshot);
    }
    Ok(snapshot)
}
