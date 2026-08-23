#![allow(async_fn_in_trait)]

use flower_host::{
    ClaimId, ClaimPendingEffectsRequest, ClaimedEffect, CommitError, CommitOutcome, DispatcherId,
    EffectClaim, EffectClaimError, ExecutionCommit, ExecutionCommitParts, ExecutionHead,
    ExecutionStore, LeaseInstant, OutboxEffect, OutboxEffectStatus, StoreError, StoredExecution,
};
use flower_kernel::{ExecutionEffect, ExecutionEvent, ExecutionRevision, ExecutionSnapshot};
use flower_plan::{
    EffectId, ExecutableWorkflowPlan, ExecutionId, PlanFingerprint, PlanReference,
    SpecificationVersion, WorkflowId,
};
use tokio::sync::Mutex;
use tokio_postgres::{Client, GenericClient, Row, Transaction, types::Json};

pub const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_execution_store.sql");

pub struct PostgresExecutionStore {
    client: Mutex<Client>,
}

impl PostgresExecutionStore {
    pub fn new(client: Client) -> Self {
        Self {
            client: Mutex::new(client),
        }
    }

    pub async fn migrate(&self) -> Result<(), StoreError> {
        self.client
            .lock()
            .await
            .batch_execute(INITIAL_MIGRATION)
            .await
            .map_err(database_error)
    }

    pub async fn verify_stored_execution(
        &self,
        plan: &ExecutableWorkflowPlan,
        execution_id: &ExecutionId,
    ) -> Result<Option<StoredExecution>, StoreError> {
        let stored = self.load(execution_id).await?;
        if let Some(stored) = &stored {
            stored.validate_consistency(execution_id)?;
            let replayed = flower_host::replay(plan, stored.events.clone()).map_err(|_| {
                StoreError::InconsistentExecution {
                    execution_id: execution_id.clone(),
                }
            })?;
            if replayed.as_ref() != Some(&stored.snapshot) {
                return Err(StoreError::InconsistentExecution {
                    execution_id: execution_id.clone(),
                });
            }
        }
        Ok(stored)
    }
}

impl ExecutionStore for PostgresExecutionStore {
    async fn load(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<Option<StoredExecution>, StoreError> {
        let client = self.client.lock().await;
        load_stored_execution(&*client, execution_id).await
    }

    async fn commit(&self, commit: ExecutionCommit) -> Result<CommitOutcome, CommitError> {
        let mut client = self.client.lock().await;
        let transaction = client.transaction().await.map_err(database_error)?;
        let outcome = commit_transaction(&transaction, commit).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }

    async fn claim_pending_effects(
        &self,
        execution_id: &ExecutionId,
        request: &ClaimPendingEffectsRequest,
    ) -> Result<Vec<ClaimedEffect>, EffectClaimError> {
        let client = self.client.lock().await;
        if !execution_exists(&*client, execution_id).await? {
            return Err(EffectClaimError::ExecutionNotFound {
                execution_id: execution_id.clone(),
            });
        }
        let limit = i64::try_from(request.maximum_count)
            .map_err(|_| EffectClaimError::Store(unavailable("claim limit exceeds i64")))?;
        let rows = client
            .query(
                "WITH candidates AS (\
                    SELECT effect_id FROM flower_execution_outbox \
                    WHERE execution_id = $1 \
                      AND (dispatch_state = 'pending' OR (dispatch_state = 'claimed' AND lease_until <= $2::text::numeric)) \
                    ORDER BY created_revision, ordinal \
                    FOR UPDATE SKIP LOCKED LIMIT $3\
                 ) \
                 UPDATE flower_execution_outbox AS outbox \
                 SET dispatch_state = 'claimed', claim_id = $4, owner_id = $5, lease_until = $6::text::numeric \
                 FROM candidates WHERE outbox.effect_id = candidates.effect_id \
                 RETURNING outbox.effect, outbox.created_revision::text, outbox.ordinal",
                &[
                    &execution_id.as_str(),
                    &request.now.0.to_string(),
                    &limit,
                    &request.claim_id.as_str(),
                    &request.owner_id.as_str(),
                    &request.lease_until.0.to_string(),
                ],
            )
            .await
            .map_err(database_claim_error)?;
        let claim = EffectClaim {
            claim_id: request.claim_id.clone(),
            owner_id: request.owner_id.clone(),
            lease_until: request.lease_until,
        };
        let mut claimed = rows
            .into_iter()
            .map(|row| {
                let effect = decode_json::<ExecutionEffect>(row.get("effect"))?;
                let revision = parse_u64(row.get("created_revision"))?;
                let ordinal: i32 = row.get("ordinal");
                Ok((
                    revision,
                    ordinal,
                    ClaimedEffect {
                        effect,
                        claim: claim.clone(),
                    },
                ))
            })
            .collect::<Result<Vec<_>, StoreError>>()
            .map_err(EffectClaimError::Store)?;
        claimed.sort_by_key(|(revision, ordinal, _)| (*revision, *ordinal));
        Ok(claimed.into_iter().map(|(_, _, effect)| effect).collect())
    }

    async fn confirm_effect_dispatched(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
        now: LeaseInstant,
    ) -> Result<(), EffectClaimError> {
        let client = self.client.lock().await;
        let changed = client
            .execute(
                "UPDATE flower_execution_outbox \
                 SET dispatch_state = 'confirmed', claim_id = NULL, owner_id = NULL, lease_until = NULL \
                 WHERE execution_id = $1 AND effect_id = $2 AND dispatch_state = 'claimed' \
                   AND claim_id = $3 AND owner_id = $4 AND lease_until = $5::text::numeric \
                   AND lease_until > $6::text::numeric",
                &[
                    &execution_id.as_str(),
                    &effect_id.as_str(),
                    &claim.claim_id.as_str(),
                    &claim.owner_id.as_str(),
                    &claim.lease_until.0.to_string(),
                    &now.0.to_string(),
                ],
            )
            .await
            .map_err(database_claim_error)?;
        if changed == 1 {
            return Ok(());
        }
        diagnose_claim_failure(&*client, execution_id, effect_id, claim, Some(now)).await
    }

    async fn release_effect_claim(
        &self,
        execution_id: &ExecutionId,
        effect_id: &EffectId,
        claim: &EffectClaim,
    ) -> Result<(), EffectClaimError> {
        let client = self.client.lock().await;
        let changed = client
            .execute(
                "UPDATE flower_execution_outbox \
                 SET dispatch_state = 'pending', claim_id = NULL, owner_id = NULL, lease_until = NULL \
                 WHERE execution_id = $1 AND effect_id = $2 AND dispatch_state = 'claimed' \
                   AND claim_id = $3 AND owner_id = $4 AND lease_until = $5::text::numeric",
                &[
                    &execution_id.as_str(),
                    &effect_id.as_str(),
                    &claim.claim_id.as_str(),
                    &claim.owner_id.as_str(),
                    &claim.lease_until.0.to_string(),
                ],
            )
            .await
            .map_err(database_claim_error)?;
        if changed == 1 {
            return Ok(());
        }
        diagnose_claim_failure(&*client, execution_id, effect_id, claim, None).await
    }
}

async fn commit_transaction(
    transaction: &Transaction<'_>,
    commit: ExecutionCommit,
) -> Result<CommitOutcome, CommitError> {
    if let Some(row) = transaction
        .query_opt(
            "SELECT event FROM flower_execution_events WHERE execution_id = $1 AND event_id = $2",
            &[
                &commit.execution_id().as_str(),
                &commit.event().event_id().as_str(),
            ],
        )
        .await
        .map_err(database_error)?
    {
        let existing = decode_json::<ExecutionEvent>(row.get("event"))?;
        if existing == *commit.event() {
            return Ok(CommitOutcome::AlreadyCommitted);
        }
        return Err(CommitError::EventIdentityConflict {
            event_id: commit.event().event_id().clone(),
        });
    }

    let existing_head = transaction
        .query_opt(
            "SELECT specification_major, specification_minor, workflow_id, plan_fingerprint, revision::text \
             FROM flower_executions WHERE execution_id = $1 FOR UPDATE",
            &[&commit.execution_id().as_str()],
        )
        .await
        .map_err(database_error)?;
    let actual_revision = existing_head
        .as_ref()
        .map(|row| parse_revision(row.get("revision")))
        .transpose()?
        .unwrap_or(ExecutionRevision(0));
    if actual_revision != commit.expected_revision() {
        return Err(CommitError::Conflict {
            expected: commit.expected_revision(),
            actual: actual_revision,
        });
    }
    if let Some(row) = &existing_head {
        let stored_reference = plan_reference_from_row(row)?;
        if stored_reference != commit.transition().snapshot.plan_reference {
            return Err(CommitError::PlanReferenceMismatch);
        }
    }

    let ExecutionCommitParts {
        execution_id,
        event,
        transition,
        ..
    } = commit.into_parts();
    let reference = &transition.snapshot.plan_reference;
    let snapshot_json = encode_json(&transition.snapshot)?;
    if existing_head.is_some() {
        let changed = transaction
            .execute(
                "UPDATE flower_executions SET revision = $2::text::numeric, snapshot = $3 \
                 WHERE execution_id = $1 AND revision = $4::text::numeric",
                &[
                    &execution_id.as_str(),
                    &transition.snapshot.revision.0.to_string(),
                    &snapshot_json,
                    &actual_revision.0.to_string(),
                ],
            )
            .await
            .map_err(database_error)?;
        if changed != 1 {
            return Err(CommitError::Conflict {
                expected: actual_revision,
                actual: load_revision(transaction, &execution_id).await?,
            });
        }
    } else {
        let inserted = transaction
            .execute(
                "INSERT INTO flower_executions \
                 (execution_id, specification_major, specification_minor, workflow_id, plan_fingerprint, revision, snapshot) \
                 VALUES ($1, $2, $3, $4, $5, $6::text::numeric, $7) \
                 ON CONFLICT (execution_id) DO NOTHING",
                &[
                    &execution_id.as_str(),
                    &i32::from(reference.specification_version.major),
                    &i32::from(reference.specification_version.minor),
                    &reference.workflow_id.as_str(),
                    &reference.fingerprint.as_str(),
                    &transition.snapshot.revision.0.to_string(),
                    &snapshot_json,
                ],
            )
            .await
            .map_err(database_error)?;
        if inserted == 0 {
            if let Some(row) = transaction
                .query_opt(
                    "SELECT event FROM flower_execution_events WHERE execution_id = $1 AND event_id = $2",
                    &[&execution_id.as_str(), &event.event_id().as_str()],
                )
                .await
                .map_err(database_error)?
            {
                let existing = decode_json::<ExecutionEvent>(row.get("event"))?;
                if existing == event {
                    return Ok(CommitOutcome::AlreadyCommitted);
                }
                return Err(CommitError::EventIdentityConflict {
                    event_id: event.event_id().clone(),
                });
            }
            let row = transaction
                .query_one(
                    "SELECT specification_major, specification_minor, workflow_id, plan_fingerprint, revision::text \
                     FROM flower_executions WHERE execution_id = $1",
                    &[&execution_id.as_str()],
                )
                .await
                .map_err(database_error)?;
            if plan_reference_from_row(&row)? != *reference {
                return Err(CommitError::PlanReferenceMismatch);
            }
            return Err(CommitError::Conflict {
                expected: ExecutionRevision(0),
                actual: parse_revision(row.get("revision"))?,
            });
        }
    }
    transaction
        .execute(
            "INSERT INTO flower_execution_events (execution_id, event_id, revision, event) \
             VALUES ($1, $2, $3::text::numeric, $4)",
            &[
                &execution_id.as_str(),
                &event.event_id().as_str(),
                &transition.snapshot.revision.0.to_string(),
                &encode_json(&event)?,
            ],
        )
        .await
        .map_err(database_error)?;
    for (ordinal, effect) in transition.effects.iter().enumerate() {
        let ordinal = i32::try_from(ordinal)
            .map_err(|_| CommitError::Store(unavailable("effect ordinal exceeds i32")))?;
        let inserted = transaction
            .execute(
                "INSERT INTO flower_execution_outbox \
                 (effect_id, execution_id, created_revision, ordinal, effect) \
                 VALUES ($1, $2, $3::text::numeric, $4, $5) ON CONFLICT (effect_id) DO NOTHING",
                &[
                    &effect.effect_id().as_str(),
                    &execution_id.as_str(),
                    &transition.snapshot.revision.0.to_string(),
                    &ordinal,
                    &encode_json(effect)?,
                ],
            )
            .await
            .map_err(database_error)?;
        if inserted != 1 {
            return Err(CommitError::EffectIdentityConflict {
                effect_id: effect.effect_id().clone(),
            });
        }
    }
    Ok(CommitOutcome::Committed)
}

async fn load_stored_execution(
    client: &impl GenericClient,
    execution_id: &ExecutionId,
) -> Result<Option<StoredExecution>, StoreError> {
    let Some(head_row) = client
        .query_opt(
            "SELECT specification_major, specification_minor, workflow_id, plan_fingerprint, revision::text, snapshot \
             FROM flower_executions WHERE execution_id = $1",
            &[&execution_id.as_str()],
        )
        .await
        .map_err(database_error)?
    else {
        return Ok(None);
    };
    let head = ExecutionHead {
        plan_reference: plan_reference_from_row(&head_row)?,
        revision: parse_revision(head_row.get("revision"))?,
    };
    let snapshot = decode_json::<ExecutionSnapshot>(head_row.get("snapshot"))?;
    let events = client
        .query(
            "SELECT event FROM flower_execution_events WHERE execution_id = $1 ORDER BY revision",
            &[&execution_id.as_str()],
        )
        .await
        .map_err(database_error)?
        .into_iter()
        .map(|row| decode_json(row.get("event")))
        .collect::<Result<Vec<_>, _>>()?;
    let outbox = client
        .query(
            "SELECT effect, dispatch_state, claim_id, owner_id, lease_until::text \
             FROM flower_execution_outbox WHERE execution_id = $1 ORDER BY created_revision, ordinal",
            &[&execution_id.as_str()],
        )
        .await
        .map_err(database_error)?
        .into_iter()
        .map(outbox_effect_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    let stored = StoredExecution {
        head,
        snapshot,
        events,
        outbox,
    };
    stored.validate_consistency(execution_id)?;
    Ok(Some(stored))
}

fn outbox_effect_from_row(row: Row) -> Result<OutboxEffect, StoreError> {
    let effect = decode_json(row.get("effect"))?;
    let status = match row.get::<_, &str>("dispatch_state") {
        "pending" => OutboxEffectStatus::Pending,
        "confirmed" => OutboxEffectStatus::Confirmed,
        "claimed" => OutboxEffectStatus::Claimed {
            claim: EffectClaim {
                claim_id: ClaimId::new(required_text(&row, "claim_id")?)
                    .map_err(|error| unavailable(error.to_string()))?,
                owner_id: DispatcherId::new(required_text(&row, "owner_id")?)
                    .map_err(|error| unavailable(error.to_string()))?,
                lease_until: LeaseInstant(parse_u64(&required_text(&row, "lease_until")?)?),
            },
        },
        value => return Err(unavailable(format!("unknown outbox state `{value}`"))),
    };
    Ok(OutboxEffect { effect, status })
}

async fn diagnose_claim_failure(
    client: &impl GenericClient,
    execution_id: &ExecutionId,
    effect_id: &EffectId,
    claim: &EffectClaim,
    now: Option<LeaseInstant>,
) -> Result<(), EffectClaimError> {
    if !execution_exists(client, execution_id).await? {
        return Err(EffectClaimError::ExecutionNotFound {
            execution_id: execution_id.clone(),
        });
    }
    let Some(row) = client
        .query_opt(
            "SELECT dispatch_state, claim_id, owner_id, lease_until::text \
             FROM flower_execution_outbox WHERE execution_id = $1 AND effect_id = $2",
            &[&execution_id.as_str(), &effect_id.as_str()],
        )
        .await
        .map_err(database_claim_error)?
    else {
        return Err(EffectClaimError::EffectNotFound {
            effect_id: effect_id.clone(),
        });
    };
    if row.get::<_, &str>("dispatch_state") != "claimed" {
        return Err(EffectClaimError::EffectNotClaimed {
            effect_id: effect_id.clone(),
        });
    }
    let stored_claim = EffectClaim {
        claim_id: ClaimId::new(required_text(&row, "claim_id").map_err(EffectClaimError::Store)?)
            .map_err(|error| EffectClaimError::Store(unavailable(error.to_string())))?,
        owner_id: DispatcherId::new(
            required_text(&row, "owner_id").map_err(EffectClaimError::Store)?,
        )
        .map_err(|error| EffectClaimError::Store(unavailable(error.to_string())))?,
        lease_until: LeaseInstant(
            parse_u64(&required_text(&row, "lease_until").map_err(EffectClaimError::Store)?)
                .map_err(EffectClaimError::Store)?,
        ),
    };
    if stored_claim != *claim {
        return Err(EffectClaimError::ClaimIdentityMismatch {
            effect_id: effect_id.clone(),
        });
    }
    if let Some(now) = now
        && now >= stored_claim.lease_until
    {
        return Err(EffectClaimError::ClaimExpired {
            effect_id: effect_id.clone(),
            lease_until: stored_claim.lease_until,
            now,
        });
    }
    Err(EffectClaimError::ClaimIdentityMismatch {
        effect_id: effect_id.clone(),
    })
}

async fn execution_exists(
    client: &impl GenericClient,
    execution_id: &ExecutionId,
) -> Result<bool, StoreError> {
    client
        .query_opt(
            "SELECT 1 FROM flower_executions WHERE execution_id = $1",
            &[&execution_id.as_str()],
        )
        .await
        .map(|row| row.is_some())
        .map_err(database_error)
}

async fn load_revision(
    client: &impl GenericClient,
    execution_id: &ExecutionId,
) -> Result<ExecutionRevision, StoreError> {
    let row = client
        .query_one(
            "SELECT revision::text FROM flower_executions WHERE execution_id = $1",
            &[&execution_id.as_str()],
        )
        .await
        .map_err(database_error)?;
    parse_revision(row.get(0))
}

fn plan_reference_from_row(row: &Row) -> Result<PlanReference, StoreError> {
    let major = u16::try_from(row.get::<_, i32>("specification_major"))
        .map_err(|_| unavailable("invalid specification major"))?;
    let minor = u16::try_from(row.get::<_, i32>("specification_minor"))
        .map_err(|_| unavailable("invalid specification minor"))?;
    Ok(PlanReference {
        specification_version: SpecificationVersion { major, minor },
        workflow_id: WorkflowId::new(row.get::<_, String>("workflow_id"))
            .map_err(|error| unavailable(error.to_string()))?,
        fingerprint: PlanFingerprint::from_sha256(row.get::<_, &str>("plan_fingerprint"))
            .map_err(|error| unavailable(error.to_string()))?,
    })
}

fn encode_json<T: serde::Serialize>(value: &T) -> Result<Json<serde_json::Value>, StoreError> {
    serde_json::to_value(value)
        .map(Json)
        .map_err(|error| unavailable(error.to_string()))
}

fn decode_json<T: serde::de::DeserializeOwned>(
    value: Json<serde_json::Value>,
) -> Result<T, StoreError> {
    serde_json::from_value(value.0).map_err(|error| unavailable(error.to_string()))
}

fn required_text(row: &Row, column: &str) -> Result<String, StoreError> {
    row.try_get::<_, Option<String>>(column)
        .map_err(database_error)?
        .ok_or_else(|| unavailable(format!("column `{column}` is unexpectedly null")))
}

fn parse_revision(value: &str) -> Result<ExecutionRevision, StoreError> {
    parse_u64(value).map(ExecutionRevision)
}

fn parse_u64(value: &str) -> Result<u64, StoreError> {
    value
        .parse()
        .map_err(|error| unavailable(format!("invalid unsigned integer `{value}`: {error}")))
}

fn unavailable(message: impl Into<String>) -> StoreError {
    StoreError::Unavailable {
        message: message.into(),
    }
}

fn database_error(error: tokio_postgres::Error) -> StoreError {
    unavailable(error.to_string())
}

fn database_claim_error(error: tokio_postgres::Error) -> EffectClaimError {
    EffectClaimError::Store(database_error(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_freezes_atomicity_and_claim_constraints() {
        for required in [
            "PRIMARY KEY (execution_id, event_id)",
            "effect_id TEXT PRIMARY KEY",
            "UNIQUE (execution_id, revision)",
            "dispatch_state IN ('pending', 'claimed', 'confirmed')",
            "claim_id IS NOT NULL",
        ] {
            assert!(INITIAL_MIGRATION.contains(required), "missing `{required}`");
        }
    }
}
