#![allow(async_fn_in_trait)]

use flower_kernel::{
    ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionSnapshot, Transition,
    TransitionError, transition,
};
use flower_plan::{EffectId, EventId, ExecutableWorkflowPlan, ExecutionId};
use std::sync::Arc;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingEffect {
    pub effect: ExecutionEffect,
    pub dispatched: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredExecution {
    pub snapshot: ExecutionSnapshot,
    pub events: Vec<ExecutionEvent>,
    pub outbox: Vec<PendingEffect>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StoreError {
    #[error("execution store is unavailable: {message}")]
    Unavailable { message: String },
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CommitError {
    #[error("execution revision conflict: expected {expected:?}, actual {actual:?}")]
    Conflict {
        expected: ExecutionRevision,
        actual: ExecutionRevision,
    },
    #[error("event id `{event_id}` was already committed")]
    DuplicateEvent { event_id: EventId },
    #[error("commit execution id does not match transition")]
    ExecutionIdMismatch,
    #[error(transparent)]
    Store(#[from] StoreError),
}

pub trait ExecutionStore: Send + Sync {
    async fn load(&self, execution_id: &ExecutionId)
    -> Result<Option<StoredExecution>, StoreError>;
    async fn commit(
        &self,
        execution_id: &ExecutionId,
        expected_revision: ExecutionRevision,
        event: ExecutionEvent,
        transition: Transition,
    ) -> Result<(), CommitError>;
    async fn mark_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
    ) -> Result<(), StoreError>;
}

impl<Store: ExecutionStore + ?Sized> ExecutionStore for Arc<Store> {
    async fn load(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<Option<StoredExecution>, StoreError> {
        (**self).load(execution_id).await
    }
    async fn commit(
        &self,
        execution_id: &ExecutionId,
        expected_revision: ExecutionRevision,
        event: ExecutionEvent,
        transition: Transition,
    ) -> Result<(), CommitError> {
        (**self)
            .commit(execution_id, expected_revision, event, transition)
            .await
    }
    async fn mark_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
    ) -> Result<(), StoreError> {
        (**self)
            .mark_effect_dispatched(execution_id, effect_id)
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

    pub async fn handle(
        &self,
        plan: &ExecutableWorkflowPlan,
        event: ExecutionEvent,
    ) -> Result<Transition, HostError> {
        let execution_id = event.execution_id().clone();
        let stored = self.store.load(&execution_id).await?;
        if let Some(stored) = &stored
            && stored
                .events
                .iter()
                .any(|existing| existing.event_id() == event.event_id())
        {
            return Err(CommitError::DuplicateEvent {
                event_id: event.event_id().clone(),
            }
            .into());
        }
        let snapshot = stored.as_ref().map(|value| &value.snapshot);
        let expected_revision = snapshot.map_or(ExecutionRevision(0), |value| value.revision);
        let next = transition(plan, snapshot, event.clone())?;
        self.store
            .commit(&execution_id, expected_revision, event, next.clone())
            .await?;
        Ok(next)
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
        for pending in stored.outbox.iter().filter(|pending| !pending.dispatched) {
            dispatcher
                .dispatch(&pending.effect)
                .await
                .map_err(DispatchError::Dispatcher)?;
            self.store
                .mark_effect_dispatched(execution_id, pending.effect.effect_id())
                .await
                .map_err(DispatchError::Store)?;
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
