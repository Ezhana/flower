# Flower Specification v0.2

## Scope

This version replaces the v0.1 node-completion model with explicit node activations and attempts. The runtime accepts only v0.2 plans. v0.1 remains historical documentation and has no runtime conversion path.

Every compiled activity contains an explicit retry policy. An activity without an input policy compiles to `max_attempts = 1`, an empty retryable failure-code set, and `backoff = none`; its first failure therefore remains terminal and emits no further effect.

## Canonical identities

Every identity below is the lowercase hexadecimal SHA-256 digest prefixed with its type name. Hash input is a sequence of fields. Each field is encoded as an unsigned 64-bit big-endian byte length followed by the field bytes. Integer field values are themselves unsigned big-endian bytes. Domain separators are encoded as the first length-prefixed field.

### NodeActivationId

Domain separator: `flower/node-activation/v2`

Fields, in order:

1. execution ID UTF-8 bytes;
2. activation revision as `u64`;
3. node ID UTF-8 bytes.

Result prefix: `activation-`.

### AttemptId

Domain separator: `flower/attempt/v2`

Fields, in order:

1. activation ID UTF-8 bytes;
2. attempt number as `u32`.

Result prefix: `attempt-`. Attempt numbers are non-zero and begin at one.

### ExecuteNodeAttempt EffectId

Domain separator: `flower/effect/execute-node-attempt/v2`

Fields, in order:

1. activation ID UTF-8 bytes;
2. attempt ID UTF-8 bytes;
3. attempt number as `u32`;
4. node ID UTF-8 bytes.

Result prefix: `effect-`.

### TimerId

Domain separator: `flower/timer/retry/v2`

Fields, in order:

1. activation ID UTF-8 bytes;
2. failed attempt ID UTF-8 bytes;
3. next attempt number as `u32`.

Result prefix: `timer-`.

### ScheduleTimer EffectId

Domain separator: `flower/effect/schedule-timer/v2`

Fields, in order:

1. timer ID UTF-8 bytes;
2. activation ID UTF-8 bytes;
3. failed attempt ID UTF-8 bytes;
4. next attempt number as `u32`.

Result prefix: `effect-`. Exact derived values are frozen by the shared fixtures.

## Retry policy and backoff

Retry is a plan property and MUST NOT be selected by a worker or loaded from mutable runtime configuration. A failed attempt is retryable only when its failure code is in the activity policy and its number is less than `max_attempts`.

Backoff is deterministic integer arithmetic:

- `none` yields zero milliseconds;
- `fixed` yields `delay_ms`;
- `exponential` yields `min(initial_delay_ms * multiplier^(failed_attempt_number - 1), maximum_delay_ms)`.

Exponential multiplication saturates at `u64::MAX` before applying the configured maximum. Floating point and jitter are forbidden. `max_attempts` and `multiplier` MUST be at least one; the exponential maximum MUST be no less than its initial delay.

## State transitions

```text
None + ExecutionStarted
-> AwaitingAttempt(first activity activation, attempt 1)
 + ExecuteNodeAttempt
```

For a direct start-to-finish plan, `ExecutionStarted` produces `Completed` and no effect.

```text
AwaitingAttempt + matching NodeAttemptSucceeded
-> AwaitingAttempt(next activity activation, attempt 1)
 + ExecuteNodeAttempt
```

The final activity success produces `Completed` and no effect.

```text
AwaitingAttempt + matching NodeAttemptFailed
-> Failed, when the code is not retryable or attempts are exhausted
-> WaitingForRetry + ScheduleTimer, otherwise
```

```text
WaitingForRetry + matching TimerFired
-> AwaitingAttempt(same activation, new attempt)
 + ExecuteNodeAttempt
```

Each retry retains the activation ID and derives a new attempt ID and execute-effect ID. A retryable failure produces exactly one timer effect. `Completed` and `Failed` are terminal. Snapshots contain only the current or last activation and attempt; attempt history exists only in the event log.

The Kernel computes only `delay_ms`; it MUST NOT read a clock. The Host persists `ScheduleTimer` through the normal outbox boundary. A timer service advances an execution only by submitting `TimerFired`. Exact duplicate events return `AlreadyCommitted`; no direct retry-wakeup operation exists.

## Validation order

After plan and snapshot integrity validation, an attempt result is checked in this normative order:

1. execution ID;
2. expected execution revision;
3. execution is awaiting an attempt;
4. activation ID;
5. attempt ID;
6. attempt number;
7. effect ID;
8. node ID.

The first failed check determines the error code. A snapshot whose activation, attempt, node, revision, or derived identity is internally inconsistent returns `invalid-snapshot` before event matching.

After plan and snapshot integrity validation, `TimerFired` is checked in this normative order:

1. execution ID;
2. expected execution revision;
3. execution is waiting for retry;
4. timer ID;
5. activation ID;
6. next attempt number.

## Durable outbox

An event commit MUST atomically advance the execution head and persist the snapshot, event, and every emitted effect intent. No subset may become visible. `(execution_id, event_id)` is unique, while effect IDs are globally unique.

Outbox delivery is at-least-once. An entry moves through `Pending`, `Claimed { claim_id, owner_id, lease_until }`, and `Confirmed`. Only the matching owner and claim identity may confirm before `lease_until`; expiration occurs when `now >= lease_until`. An expired or explicitly released claim becomes claimable again. Concurrent claimers MUST NOT own the same effect during one active lease.

Time values are explicit Store/Host inputs and never Kernel inputs. A successful external dispatch followed by a crash before confirmation may be delivered again, so executors MUST deduplicate globally by EffectId. Flower does not claim exactly-once delivery.
