CREATE TABLE IF NOT EXISTS flower_executions (
    execution_id TEXT PRIMARY KEY,
    specification_major INTEGER NOT NULL CHECK (specification_major BETWEEN 0 AND 65535),
    specification_minor INTEGER NOT NULL CHECK (specification_minor BETWEEN 0 AND 65535),
    workflow_id TEXT NOT NULL,
    plan_fingerprint CHAR(64) NOT NULL,
    revision NUMERIC(20, 0) NOT NULL CHECK (revision >= 0),
    snapshot JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS flower_execution_events (
    execution_id TEXT NOT NULL REFERENCES flower_executions(execution_id) ON DELETE CASCADE,
    event_id TEXT NOT NULL,
    revision NUMERIC(20, 0) NOT NULL CHECK (revision > 0),
    event JSONB NOT NULL,
    PRIMARY KEY (execution_id, event_id),
    UNIQUE (execution_id, revision)
);

CREATE TABLE IF NOT EXISTS flower_execution_outbox (
    effect_id TEXT PRIMARY KEY,
    execution_id TEXT NOT NULL REFERENCES flower_executions(execution_id) ON DELETE CASCADE,
    created_revision NUMERIC(20, 0) NOT NULL CHECK (created_revision > 0),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    effect JSONB NOT NULL,
    dispatch_state TEXT NOT NULL DEFAULT 'pending'
        CHECK (dispatch_state IN ('pending', 'claimed', 'confirmed')),
    claim_id TEXT,
    owner_id TEXT,
    lease_until NUMERIC(20, 0),
    UNIQUE (execution_id, created_revision, ordinal),
    CHECK (
        (dispatch_state = 'claimed' AND claim_id IS NOT NULL AND owner_id IS NOT NULL AND lease_until IS NOT NULL)
        OR
        (dispatch_state IN ('pending', 'confirmed') AND claim_id IS NULL AND owner_id IS NULL AND lease_until IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS flower_execution_outbox_claimable
    ON flower_execution_outbox (execution_id, dispatch_state, lease_until, created_revision, ordinal);
