#![allow(async_fn_in_trait)]

use flower_kernel::{
    ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionSnapshot, Transition,
    TransitionError, transition, validate_snapshot,
};
use flower_plan::{EffectId, EventId, ExecutableWorkflowPlan, ExecutionId, PlanReference};
use std::{collections::BTreeSet, sync::Arc};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingEffect {
    pub effect: ExecutionEffect,
    pub dispatched: bool,
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
    pub outbox: Vec<PendingEffect>,
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
        let event_count = u64::try_from(self.events.len()).ok();
        if self.snapshot.execution_id != *execution_id
            || self.snapshot.plan_reference != self.head.plan_reference
            || self.snapshot.revision != self.head.revision
            || event_count != Some(self.head.revision.0)
            || !first_event_matches
            || !events_are_consistent
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
            | (ExecutionEvent::NodeCompleted { .. }, 1..) => {}
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
pub enum ConfirmEffectError {
    #[error("execution `{execution_id}` does not exist")]
    ExecutionNotFound { execution_id: ExecutionId },
    #[error("effect `{effect_id}` does not exist in the execution outbox")]
    EffectNotFound { effect_id: EffectId },
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub trait ExecutionStore: Send + Sync {
    async fn load(&self, execution_id: &ExecutionId)
    -> Result<Option<StoredExecution>, StoreError>;
    async fn commit(&self, commit: ExecutionCommit) -> Result<CommitOutcome, CommitError>;
    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
    ) -> Result<(), ConfirmEffectError>;
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
    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
    ) -> Result<(), ConfirmEffectError> {
        (**self)
            .confirm_effect_dispatched(execution_id, effect_id)
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

pub struct WorkflowHost<Store> {
    store: Store,
}

impl<Store: ExecutionStore> WorkflowHost<Store> {
    pub fn new(store: Store) -> Self {
        Self { store }
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
        dispatcher: &Dispatcher,
    ) -> Result<(), DispatchError<Dispatcher::Error>> {
        let Some(stored) = self
            .store
            .load(execution_id)
            .await
            .map_err(DispatchError::Store)?
        else {
            return Ok(());
        };
        stored
            .validate_consistency(execution_id)
            .map_err(DispatchError::Store)?;
        for pending in stored.outbox.iter().filter(|pending| !pending.dispatched) {
            dispatcher
                .dispatch(&pending.effect)
                .await
                .map_err(DispatchError::Dispatcher)?;
            self.store
                .confirm_effect_dispatched(execution_id, pending.effect.effect_id())
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
    #[error(transparent)]
    Store(StoreError),
    #[error(transparent)]
    Confirm(ConfirmEffectError),
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
        ExecutionEvent::NodeCompleted { .. } => {
            return Err(TransitionError::ExecutionNotStarted);
        }
    }

    let mut snapshot = Some(transition(plan, None, first_event)?.snapshot);
    for event in events {
        snapshot = Some(transition(plan, snapshot.as_ref(), event)?.snapshot);
    }
    Ok(snapshot)
}
