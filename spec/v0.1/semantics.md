# Flower Specification v0.1

## Scope

v0.1 supports exactly one start node, exactly one finish node and zero or more Activity nodes on one start-to-finish path. Gateway and edge condition are invalid capabilities.

## Plan

Compiler MUST reject duplicate identifiers, missing endpoints, invalid degrees, cycles and disconnected nodes. Input node/edge order MUST NOT affect normalized node order or fingerprint. Kernel MUST reject a plan that fails its integrity check.

## Payload

Payload is an opaque media type and byte sequence. Kernel MUST NOT interpret its contents.

## Identity and revision

Every execution, event and effect has a stable identifier. `PlanReference` is the immutable tuple `(specification_version, workflow_id, fingerprint)`. `ExecutionStarted` MUST carry the exact reference of the executable plan, and every snapshot MUST retain that reference. Revision starts at one after `ExecutionStarted` and increases exactly once for every accepted completion event.

An event log is bound by its first `ExecutionStarted` fact. Replay MUST verify that fact's complete `PlanReference` against the supplied plan before applying the first transition. A mismatch returns `plan-reference-mismatch`; callers cannot replay an event log against another workflow merely because its node IDs happen to match.

### ExecuteNode EffectId algorithm

The v0.1 `ExecuteNode` EffectId is `"effect-" + lower_hex(SHA-256(canonical_input))`, where `canonical_input` is the following byte sequence:

```text
ASCII("flower.effect-id.v0.1") || 0x00
|| U64_BE(byte_length(UTF8(execution_id))) || UTF8(execution_id)
|| U64_BE(revision)
|| U64_BE(byte_length(UTF8(node_id))) || UTF8(node_id)
```

Lengths count UTF-8 bytes, not characters. `U64_BE` is an unsigned 64-bit big-endian integer. The domain separator, length prefixes, byte order, `effect-` prefix and lowercase hexadecimal encoding are normative.

Golden values:

```text
(execution_id="tenant", revision=1, node_id="node.2.job")
  -> effect-06c8f27d9de5825234bbf9754c13a557b394839002c05df3a7f7bb74af956655

(execution_id="tenant.1.node", revision=2, node_id="job")
  -> effect-156aba9fe41440419600d77ec482056ecfa03c671a3770c6d32d243fc4fe655e
```

Kernel MUST NOT use delimiter concatenation, clocks, randomness, I/O or process-local state.

## Transition

`None + ExecutionStarted` advances to the first Activity or directly completes a start-to-finish plan. A NodeCompleted event is accepted only when execution ID, expected revision, node ID and effect ID match the snapshot. An accepted completion advances to the next Activity or finish.

Completed executions reject all later events. Duplicate completions never emit another Effect. Invalid external data returns a stable error code and MUST NOT panic.

## Persistence

Host commit atomically stores the new snapshot, consumed event, new revision and Effect intents. Dispatch occurs only after commit. Stores use optimistic revision comparison. Replay validates the initial plan binding, then applies committed events in order through the same Kernel function.

## Conformance fixtures

Every JSON fixture conforms to `fixture-schema.json` and declares `schema_version = "flower.conformance/v0.1"`. Runners MUST deserialize the schema into typed values, reject unknown fields and unknown variants, and compare the full compile plan or diagnostics plus every declared transition snapshot, effect list or engine error. A runner MUST NOT infer behavior from the fixture name.
