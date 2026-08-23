# Flower Specification v0.1

## Scope

v0.1 supports exactly one start node, exactly one finish node and zero or more Activity nodes on one start-to-finish path. Gateway and edge condition are invalid capabilities.

## Plan

Compiler MUST reject duplicate identifiers, missing endpoints, invalid degrees, cycles and disconnected nodes. Input node/edge order MUST NOT affect normalized node order or fingerprint. Kernel MUST reject a plan that fails its integrity check.

## Payload

Payload is an opaque media type and byte sequence. Kernel MUST NOT interpret its contents.

## Identity and revision

Every execution, event and effect has a stable identifier. `ExecutionId` and `EffectId` MUST be globally unique. `EventId` MUST be unique within one Execution; the same textual EventId MAY be used by another Execution. Within an Execution, replaying the exact same event returns `AlreadyCommitted`, while reusing its EventId for different event content MUST fail with `event-identity-conflict`.

`PlanReference` is the immutable tuple `(specification_version, workflow_id, fingerprint)`. `ExecutionStarted` MUST carry the exact reference of the executable plan. Every snapshot and store head MUST retain that same reference. Revision starts at one after `ExecutionStarted` and increases exactly once for every accepted completion event.

An event log is bound by its first `ExecutionStarted` fact. Replay MUST verify that fact's complete `PlanReference` against the supplied plan before applying the first transition. A mismatch returns `plan-reference-mismatch`; callers cannot replay an event log against another workflow merely because its node IDs happen to match.

### ExecuteNode EffectId algorithm

The v0.1 `ExecuteNode` EffectId is `"effect-" + lower_hex(SHA-256(canonical_input))`, where `canonical_input` is the following byte sequence:

```text
ASCII("flower/effect/v1")
|| LP(ASCII("execute-node"))
|| LP(UTF8(execution_id))
|| LP(U64_BE(revision))
|| LP(UTF8(node_id))
```

`LP(value)` is `U64_BE(byte_length(value)) || value`. String lengths count UTF-8 bytes, not characters. `U64_BE` is an unsigned 64-bit big-endian integer. The domain separator has no terminator. The Effect kind, every derivation field, length prefixes, field order, byte order, `effect-` prefix and lowercase hexadecimal encoding are normative.

Golden values:

```text
(execution_id="tenant", revision=1, node_id="node.2.job")
  -> effect-a2807a32336886e70797326d3a6e7645c30336c87f8932b8499bcefcc6899b29

(execution_id="tenant.1.node", revision=2, node_id="job")
  -> effect-dc435cdd8f81eeeeb8f01981e8b2b9dbc2ce9efa1d223eb5d937b74ccb3d8965
```

Kernel MUST NOT use delimiter concatenation, clocks, randomness, I/O or process-local state.

## Transition

`None + ExecutionStarted` advances to the first Activity or directly completes a start-to-finish plan. A NodeCompleted event is accepted only when execution ID, expected revision, node ID and effect ID match the snapshot. An accepted completion advances to the next Activity or finish.

Completed executions reject all later events. Duplicate completions never emit another Effect. Invalid external data returns a stable error code and MUST NOT panic.

## Persistence

Host commit atomically stores one `ExecutionCommit`: the new snapshot, consumed event, new store head/revision and Effect intents. Dispatch occurs only after commit. Stores use optimistic revision comparison. An exact duplicate event returns `AlreadyCommitted` instead of an infrastructure error. Confirming an Effect absent from the execution outbox MUST fail.

Outbox delivery is at-least-once. A dispatcher can exit after the external operation succeeds but before confirmation is persisted, so the same Effect can be delivered again. Every external executor MUST therefore be idempotent by the globally unique `EffectId`.

Replay validates the initial plan binding before applying its first event, then applies committed events in order through the same Kernel function. The replayed snapshot, stored snapshot and stored head MUST agree on `PlanReference` and revision.

## Conformance fixtures

Every JSON fixture conforms to `fixture-schema.json` and declares `schema_version = "flower.conformance/v0.1"`. Runners MUST deserialize the schema into typed values, reject unknown fields and unknown variants, and compare the full compile plan or diagnostics plus every declared transition snapshot, effect list or engine error. A runner MUST NOT infer behavior from the fixture name.
